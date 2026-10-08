//! `/v1/chat/completions`: relayed as it is, streamed, or, with web search
//! on, run as a loop in which the model may call a tool before it answers.
//!
//! ```text
//!   client ── request ──► server ── stream ──► llama-server
//!      ▲                    │  tool call: web_search("…", time_range)
//!      │                    │             fetch_page_content_recursive(url)
//!      │                    ▼
//!      │        SearXNG ── results ──► the model, next round
//!      │        a page  ── text ──────►
//!      └──── every token, plus a {"thor":{"search":…}} or {"thor":{"read":…}} event
//! ```
//!
//! The first round is sent with `tool_choice: "required"`, so both models look
//! something up before they answer; later rounds go back to `auto`. A model that
//! writes a call into the text instead of into `tool_calls` is understood too:
//! [`crate::tooltext`] takes it out of the answer and hands it to the loop.

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use crate::{
    address::Url,
    config::Upstreams,
    error::{AgentError, Outcome},
    fetch::{self, Allowed},
    http::Web,
    paths,
    research::{Kind, Ledger, MAX_QUERIES_PER_CALL, MAX_READS, MAX_SEARCHES},
    response::{self, DONE, EVENT_PREFIX},
    search::{self, SearchResult, TimeRange},
    tooltext::Sieve,
    upstream::Endpoint,
};

/// Tool rounds per answer. Deep research is not round count — it is the
/// widening, the budget and the ledger below — but a hunt with sub-questions
/// does need room, so there are eight.
const MAX_ROUNDS: usize = 8;

/// Rounds without tools at the end. The model usually answers on the first; the
/// others are there for the ways it fails: a round that is only a tool call,
/// which is run, and a round with no text at all.
const ANSWER_ROUNDS: usize = 3;

/// Tool calls kept per round; more are ignored.
const MAX_CALLS: usize = 8;

/// The most sources named in the note before the answer round.
const MAX_CITED: usize = 40;

/// The page's switch for web search; removed before the model sees the request.
const WEB_SEARCH_SWITCH: &str = "thor_web_search";

/// The line added under the switch. It asks for a plan, names the two kinds
/// that find postings and the people behind them, and asks for citations.
const SEARCH_HINT: &str = concat!(
    "Web research is available, and answering without it is a guess. Plan first: break the ",
    "question into the sub-questions that have to be true for the answer, then run one ",
    "web_search per sub-question, using its `queries` list to search several at once. Use ",
    "kind jobs for postings and kind people for the recruiter and the hiring manager ",
    "behind them — people posts are often where a job is first mentioned. Set time_range to ",
    "day, week, month or year when the answer depends on what is recent. Read the most ",
    "promising results with fetch_page_content_recursive before you decide, and follow a ",
    "posting to the company's own page. Cite what you use with the [n] number each source is ",
    "given, and answer with headings when the answer has parts."
);

/// The line added as the last user turn before the answer rounds. A model that
/// kept calling tools in the system line's words answers this one.
const ANSWER_ASK: &str = concat!(
    "Write the answer now, in plain text, from the sources above: the facts, the ",
    "dates, the companies and the people. Cite each claim with its [n] number. ",
    "Do not call a tool."
);

/// The line added before the answer rounds, when the model must stop calling
/// tools and write the answer.
const ANSWER_NUDGE: &str = concat!(
    "The tool rounds are over. Write the answer now, in plain text, from the sources ",
    "above: the facts, the dates, the companies, and the people when the question is ",
    "about hiring. Cite each claim with its [n] number. Do not write a tool call."
);

/// The line added when the model writes a tool call in the answer rounds, where
/// no tool is offered and none runs. The call is dropped and the model is asked
/// again, because a call is not an answer and running it is how a model that
/// keeps calling tools never writes one.
const ANSWER_AGAIN: &str = concat!(
    "That was a tool call, and the tools are closed for this answer. Nothing ran. ",
    "Write the answer in plain text now, from the sources above, citing each claim ",
    "with its [n] number."
);

/// The most sources named in the last-resort note, which is all the page shows
/// when no round wrote a word.
const MAX_LISTED: usize = 10;

/// The request field asking for a streamed answer.
const STREAM: &str = "stream";

/// The request field naming the model, which picks the engine.
const MODEL: &str = "model";

/// The request field listing the tools the model may call.
const TOOLS: &str = "tools";

/// The request field forcing or allowing a tool call.
const TOOL_CHOICE: &str = "tool_choice";

/// The `tool_choice` value that makes the model call a tool.
const REQUIRED: &str = "required";

/// The request field holding the conversation.
const MESSAGES: &str = "messages";

/// A chat request's top-level fields.
type Fields = Map<String, Value>;

/// How a chat request is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Passed to the model server and its answer copied back as it is.
    Relay,
    /// Streamed to the client as events.
    Stream,
    /// Streamed, with tools available.
    Search,
}

impl Mode {
    fn of(web_search: bool, streamed: bool) -> Mode {
        match (web_search, streamed) {
            (true, _) => Mode::Search,
            (false, true) => Mode::Stream,
            (false, false) => Mode::Relay,
        }
    }
}

/// The tools the model may call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// Search the web through SearXNG.
    WebSearch,
    /// Read a page, and its same-site links, as text.
    FetchPage,
}

impl Tool {
    /// Every tool, in the order the model is told about them.
    const ALL: [Tool; 2] = [Tool::WebSearch, Tool::FetchPage];

    fn name(self) -> &'static str {
        match self {
            Tool::WebSearch => "web_search",
            Tool::FetchPage => "fetch_page_content_recursive",
        }
    }

    fn named(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// The tool as the model sees it: name, purpose, and arguments.
    fn definition(self) -> Value {
        let (description, parameters) = match self {
            Tool::WebSearch => (
                concat!(
                    "Research the web. Returns titles, addresses, dates and short snippets, each ",
                    "with a number to cite. `query` is the sub-question to search; `queries` ",
                    "adds up to three more, so one call can fan out over the parts of a ",
                    "question. `kind` widens the search: \"jobs\" looks for postings, ",
                    "\"people\" for the recruiter and hiring manager behind one, whose posts ",
                    "often mention a role first. `time_range` keeps results recent: day, ",
                    "week, month or year."
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "The sub-question to search for" },
                        "queries": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Up to three more sub-questions to search in the same call"
                        },
                        "kind": { "type": "string", "enum": Kind::ALL.map(Kind::as_str), "description": "How to widen the search" },
                        "time_range": { "type": "string", "enum": ["day", "week", "month", "year"], "description": "Keep results no older than this" }
                    },
                    "required": ["query"],
                }),
            ),
            Tool::FetchPage => (
                "Read a web page as plain text, following the page's own links up to two hops, so the answer can use the page itself and not only a snippet. Only an https address from this answer's search results, or one the user wrote, can be opened.",
                json!({
                    "type": "object",
                    "properties": { "url": { "type": "string", "description": "An https address from this answer's search results" } },
                    "required": ["url"],
                }),
            ),
        };
        json!({ "type": "function", "function": { "name": self.name(), "description": description, "parameters": parameters } })
    }
}

/// The arguments of one `web_search` call: a sub-question, more sub-questions,
/// how to widen them, and how recent the answers must be.
#[derive(Deserialize)]
struct SearchArguments {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    queries: Option<Vec<String>>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    time_range: Option<String>,
}

/// One `web_search` call, worked out: the queries to run and the recency asked
/// for. The queries are already widened by `kind` and capped.
#[derive(Debug, PartialEq, Eq)]
struct SearchPlan {
    /// The searches to run, in order.
    queries: Vec<String>,
    /// The recency to ask SearXNG for.
    range: Option<TimeRange>,
}

/// The arguments of one `fetch_page_content_recursive` call.
#[derive(Deserialize)]
struct FetchArguments {
    url: String,
}

/// One streamed chunk from llama-server, as far as this server reads it.
#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
}

/// What one chunk adds to the reply: text, pieces of tool calls, or both.
#[derive(Deserialize, Default)]
struct Delta {
    content: Option<String>,
    tool_calls: Option<Vec<CallPiece>>,
}

/// A piece of a tool call; the id, name and arguments arrive in parts.
#[derive(Deserialize)]
struct CallPiece {
    #[serde(default)]
    index: usize,
    id: Option<String>,
    function: Option<FunctionPiece>,
}

#[derive(Deserialize)]
struct FunctionPiece {
    name: Option<String>,
    arguments: Option<String>,
}

/// One tool call, assembled from its pieces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// What one streamed round produced besides the tokens already forwarded.
#[derive(Debug, Default, PartialEq, Eq)]
struct Round {
    content: String,
    calls: Vec<ToolCall>,
}

/// A message this server adds to the conversation, in OpenAI's shape.
#[derive(Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
enum Added<'a> {
    /// The model's turn that asked for tools.
    Assistant {
        content: &'a str,
        tool_calls: Vec<CallRecord<'a>>,
    },
    /// One tool's result.
    Tool {
        tool_call_id: &'a str,
        content: &'a str,
    },
}

/// A tool call as it is written back into the conversation.
#[derive(Serialize)]
struct CallRecord<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: FunctionRecord<'a>,
}

#[derive(Serialize)]
struct FunctionRecord<'a> {
    name: &'a str,
    arguments: &'a str,
}

/// An event this server adds to the stream, under a `thor` key so the page
/// can tell it from the model's own events.
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum ThorEvent<'a> {
    /// A search ran: the query and the sources it found.
    Search {
        query: &'a str,
        results: Vec<Source<'a>>,
    },
    /// A page was read: its address and title.
    Read { url: &'a str, title: &'a str },
    /// Something failed after the stream started.
    Error(String),
}

#[derive(Serialize)]
struct Source<'a> {
    title: &'a str,
    url: &'a str,
}

/// Answers one chat request on `client`.
///
/// # Errors
///
/// `AgentError::BadRequest` when the body isn't a JSON object, and upstream
/// or I/O errors before the answer starts. Errors after an event stream has
/// started are sent to the client as a `{"thor":{"error":…}}` event instead.
pub fn answer(client: &mut dyn Write, body: &[u8], upstreams: &Upstreams) -> Outcome {
    let Value::Object(mut fields) = serde_json::from_slice(body)? else {
        return Err(AgentError::bad_request("the body must be a JSON object"));
    };

    let web_search = fields
        .remove(WEB_SEARCH_SWITCH)
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let streamed = fields.get(STREAM).and_then(Value::as_bool).unwrap_or(false);
    let model = upstreams.serving(fields.get(MODEL).and_then(Value::as_str));

    match Mode::of(web_search, streamed) {
        Mode::Relay => model
            .post(paths::CHAT_COMPLETIONS, &serde_json::to_vec(&fields)?)?
            .relay(client),
        Mode::Stream => as_events(client, &mut |client| {
            stream_round(client, &fields, model).map(drop)
        }),
        Mode::Search => {
            fields.insert(STREAM.to_string(), Value::Bool(true));
            nudge_to_search(&mut fields);
            as_events(client, &mut |client| {
                search_loop(client, fields.clone(), upstreams, model)
            })
        }
    }
}

/// Runs `body` inside an event stream: an error becomes an error event, and
/// the stream always ends with `[DONE]`.
fn as_events(client: &mut dyn Write, body: &mut dyn FnMut(&mut dyn Write) -> Outcome) -> Outcome {
    response::start_events(client)?;

    if let Err(error) = body(client) {
        send_thor(client, &ThorEvent::Error(error.to_string()))?;
    }
    response::send_event(client, DONE)
}

fn send_thor(client: &mut dyn Write, event: &ThorEvent) -> Outcome {
    response::send_event(client, &json!({ "thor": event }).to_string())
}

/// Asks the model, runs the tools it calls, and asks again, until it answers
/// without a tool call or the tool rounds run out.
///
/// The first round requires a call, so both models look something up before
/// they answer; later rounds leave the choice to the model. A call that repeats
/// one already run is answered with a nudge instead of run again, so a model
/// that loops cannot loop forever. [`Ledger`] holds every source found, gives it
/// a number to cite, and holds the search and page budgets; the numbers go to
/// the model again before the answer rounds.
///
/// After the tool rounds the answer rounds run with no tools at all, and there
/// no call runs: a call written then is dropped and the model is asked again
/// ([`ANSWER_AGAIN`]). A model that keeps calling tools is exactly the one that
/// otherwise never writes a word, so the answer phase counts only text. When a
/// round writes text beside a call, the text is the answer and the call is not
/// run. Only when no round writes any text does the note name the sources the
/// ledger found, so the page still shows what the search turned up.
fn search_loop(
    client: &mut dyn Write,
    mut fields: Fields,
    upstreams: &Upstreams,
    model: &Endpoint,
) -> Outcome {
    let mut allowed = Allowed::from_text(&user_text(&fields));
    let mut ran: Vec<String> = Vec::new();
    let mut ledger = Ledger::new();

    for round_number in 1..=MAX_ROUNDS {
        offer_tools(&mut fields, true);
        require_tool(&mut fields, round_number == 1);
        let round = stream_round(client, &fields, model)?;

        if round.calls.is_empty() {
            return Ok(());
        }

        let results = run_calls(
            client,
            &round.calls,
            upstreams,
            &mut allowed,
            &mut ran,
            &mut ledger,
        )?;
        append_round(&mut fields, &round, &results)?;
        if ledger.searches_left(MAX_SEARCHES) == 0 {
            break;
        }
    }

    offer_tools(&mut fields, false);
    require_tool(&mut fields, false);
    retract_system(&mut fields, SEARCH_HINT);
    append_system(&mut fields, ANSWER_NUDGE);
    if !ledger.is_empty() {
        append_system(
            &mut fields,
            &format!(
                "Sources found, with the numbers to cite:\n{}",
                ledger.list(MAX_CITED)
            ),
        );
    }
    append_user(&mut fields, ANSWER_ASK);

    for _ in 0..ANSWER_ROUNDS {
        let round = stream_round(client, &fields, model)?;

        if !round.content.trim().is_empty() {
            return Ok(());
        }
        if round.calls.is_empty() {
            continue;
        }

        append_user(&mut fields, ANSWER_AGAIN);
    }

    let note = if ledger.is_empty() {
        "The tool rounds are over and no round wrote an answer. Ask again, or narrow the question."
            .to_string()
    } else {
        format!(
            "The tool rounds are over and no round wrote an answer. These are the sources the \
             search turned up:\n{}",
            ledger.list(MAX_LISTED)
        )
    };
    response::send_event(
        client,
        &json!({ "choices": [{ "delta": { "content": note } }] }).to_string(),
    )?;
    Ok(())
}

/// Runs a round's tool calls, skipping any that already ran.
fn run_calls(
    client: &mut dyn Write,
    calls: &[ToolCall],
    upstreams: &Upstreams,
    allowed: &mut Allowed,
    ran: &mut Vec<String>,
    ledger: &mut Ledger,
) -> Outcome<Vec<String>> {
    calls
        .iter()
        .map(|call| {
            let signature = format!("{}\u{1}{}", call.name, call.arguments);
            if ran.contains(&signature) {
                return Ok("That tool call already ran in this answer. Use its result.".to_string());
            }
            ran.push(signature);
            run_tool(client, call, upstreams, allowed, ledger)
        })
        .collect()
}

/// The text of the user's own messages: one of the two places a fetchable
/// address may come from, the other being a search result.
fn user_text(fields: &Fields) -> String {
    fields
        .get(MESSAGES)
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
                .filter_map(|message| message.get("content").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Adds [`SEARCH_HINT`] to the conversation, merging it into an existing
/// system message so the request keeps a single system turn.
fn nudge_to_search(fields: &mut Fields) {
    append_system(fields, SEARCH_HINT);
}

/// Adds `line` to the conversation's one system message, making it when there
/// is none, so the request never grows a second system turn.
fn append_system(fields: &mut Fields, line: &str) {
    let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) else {
        return;
    };

    let first_is_system = messages
        .first()
        .is_some_and(|message| message.get("role").and_then(Value::as_str) == Some("system"));

    if !first_is_system {
        messages.insert(0, json!({ "role": "system", "content": line }));

        return;
    }

    let merged = messages[0]
        .get("content")
        .and_then(Value::as_str)
        .map(|content| format!("{content}\n\n{line}"));

    if let Some(content) = merged {
        messages[0]["content"] = Value::String(content);
    }
}

/// Adds `line` as the last user turn, which is where a model looks for what to
/// do next.
fn append_user(fields: &mut Fields, line: &str) {
    let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) else {
        return;
    };
    messages.push(json!({ "role": "user", "content": line }));
}

/// Takes `line` back out of the system message that [`append_system`] put it in.
///
/// The search hint tells the model to plan sub-questions and call `web_search`,
/// and it stays in the system turn for the tool rounds. Left there for the
/// answer rounds it outranks the ask: a model that is told to search in its
/// system line writes a tool call even when the request offers no tools, which
/// is the loop that ends with no answer at all.
fn retract_system(fields: &mut Fields, line: &str) {
    let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) else {
        return;
    };
    let Some(system) = messages
        .first_mut()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("system"))
    else {
        return;
    };
    let Some(content) = system.get("content").and_then(Value::as_str) else {
        return;
    };

    let after = content.replacen(&format!("{line}\n\n"), "", 1);
    let after = after.replacen(&format!("\n\n{line}"), "", 1);
    let after = if after == line { String::new() } else { after };
    system["content"] = Value::String(after);
}

fn offer_tools(fields: &mut Fields, offered: bool) {
    if offered {
        fields.insert(
            TOOLS.to_string(),
            Tool::ALL.map(Tool::definition).into_iter().collect(),
        );
    } else {
        fields.remove(TOOLS);
    }
}

/// Sets `tool_choice: "required"` on the first round, and clears it after, so
/// the model has to call a tool once and then chooses for itself.
fn require_tool(fields: &mut Fields, required: bool) {
    if required {
        fields.insert(TOOL_CHOICE.to_string(), Value::String(REQUIRED.to_string()));
    } else {
        fields.remove(TOOL_CHOICE);
    }
}

/// Adds the model's tool calls and their results to the conversation.
fn append_round(fields: &mut Fields, round: &Round, results: &[String]) -> Outcome {
    let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) else {
        return Err(AgentError::bad_request("messages must be a list"));
    };

    messages.push(serde_json::to_value(round.as_message())?);

    for (call, result) in round.calls.iter().zip(results) {
        messages.push(serde_json::to_value(Added::Tool {
            tool_call_id: &call.id,
            content: result,
        })?);
    }
    Ok(())
}

/// Runs one tool call; its result is text for the model.
fn run_tool(
    client: &mut dyn Write,
    call: &ToolCall,
    upstreams: &Upstreams,
    allowed: &mut Allowed,
    ledger: &mut Ledger,
) -> Outcome<String> {
    match Tool::named(&call.name) {
        Some(Tool::WebSearch) => web_search(client, call, &upstreams.search, allowed, ledger),
        Some(Tool::FetchPage) => fetch_page(client, call, &*upstreams.web, allowed, ledger),
        None => Ok(format!("Unknown tool {}.", call.name)),
    }
}

/// A research round: one search per sub-question the model asked for, widened by
/// `kind` when it asked for one. Its sources also become the addresses this
/// answer may read a page from, and every source keeps one number to cite.
fn web_search(
    client: &mut dyn Write,
    call: &ToolCall,
    searxng: &Endpoint,
    allowed: &mut Allowed,
    ledger: &mut Ledger,
) -> Outcome<String> {
    let Some(plan) = call.search_plan() else {
        return Ok("The search needs a non-empty query.".to_string());
    };

    let mut blocks: Vec<String> = Vec::new();
    for query in &plan.queries {
        if ledger.searches_left(MAX_SEARCHES) == 0 {
            blocks.push(format!(
                "The search budget is spent ({MAX_SEARCHES} searches). Answer with what you have."
            ));
            break;
        }
        ledger.count_search();

        let hits = match search::search(searxng, query, plan.range) {
            Ok(hits) => hits,
            Err(error) => {
                blocks.push(format!("The search \"{query}\" failed: {error}"));
                continue;
            }
        };
        for result in &hits.results {
            allowed.add_url(&result.url);
        }
        send_thor(
            client,
            &ThorEvent::Search {
                query,
                results: sources(&hits.results),
            },
        )?;

        blocks.push(search_block(query, &hits, ledger));
    }

    blocks.push(format!(
        "Searches used {} of {MAX_SEARCHES}; pages read {} of {MAX_READS}; sources numbered 1 to {}.",
        ledger.searches(),
        ledger.reads(),
        ledger.len()
    ));
    Ok(blocks.join("\n\n"))
}

/// One query's results, under the numbers the ledger gave them, and the engines
/// that did not answer. "No results" from three dead engines is not the same as
/// "nothing exists", and the model is told which it is.
fn search_block(query: &str, hits: &search::Hits, ledger: &mut Ledger) -> String {
    let mut block = if hits.results.is_empty() {
        format!("Query: {query}\nNo results.")
    } else {
        let entries: Vec<String> = hits
            .results
            .iter()
            .map(|result| {
                let number = ledger.add(result);
                search::as_entry(result, number)
            })
            .collect();
        format!("Query: {query}\n{}", entries.join("\n\n"))
    };

    if !hits.down.is_empty() {
        block.push_str(&format!(
            "\nEngines that did not answer: {}.",
            hits.down.join(", ")
        ));
    }
    block
}

/// Reads one page and its same-site links, up to what is left of the page
/// budget. Rule 1 of the fetch design is enforced inside
/// [`fetch::read_recursive`], and every page read is sent to the page so the
/// answer can list its sources.
fn fetch_page(
    client: &mut dyn Write,
    call: &ToolCall,
    web: &dyn Web,
    allowed: &Allowed,
    ledger: &mut Ledger,
) -> Outcome<String> {
    let Some(raw) = call.url_argument() else {
        return Ok("The fetch needs an address.".to_string());
    };
    let url = match Url::parse(&raw) {
        Ok(url) => url,
        Err(error) => return Ok(error.to_string()),
    };
    let left = ledger.reads_left(MAX_READS);
    if left == 0 {
        return Ok(format!(
            "The page budget is spent ({MAX_READS} pages). Answer with what you have."
        ));
    }
    let report = match fetch::read_recursive(web, &url, allowed, left.min(fetch::MAX_PAGES)) {
        Ok(report) => report,
        Err(error) => return Ok(format!("Could not read the page: {error}")),
    };
    ledger.count_reads(report.pages.len());
    for page in &report.pages {
        ledger.add_page(&page.url, &page.title);
        let event = ThorEvent::Read {
            url: &page.url,
            title: &page.title,
        };
        send_thor(client, &event)?;
    }
    Ok(report.text)
}

fn sources(results: &[SearchResult]) -> Vec<Source<'_>> {
    results
        .iter()
        .map(|result| Source {
            title: &result.title,
            url: &result.url,
        })
        .collect()
}

/// Streams one model reply to the client, collecting any tool calls. A call the
/// model writes into the text is taken out by [`Sieve`] and runs like a
/// structured one, and the reader never sees the tags.
fn stream_round(client: &mut dyn Write, fields: &Fields, model: &Endpoint) -> Outcome<Round> {
    let response = model.post(paths::CHAT_COMPLETIONS, &serde_json::to_vec(fields)?)?;

    if !response.is_ok() {
        let status = response.status;

        return Err(AgentError::Upstream(format!(
            "model server returned {status}: {}",
            response.text()?
        )));
    }

    let mut round = Round::default();
    let mut sieve = Sieve::new();

    for line in response.body.lines() {
        let line = line?;
        let Some(data) = line.strip_prefix(EVENT_PREFIX).map(str::trim) else {
            continue;
        };

        if data == DONE {
            break;
        }

        let mut value: Value = serde_json::from_str(data)?;
        if let Some(event) = forward(&mut value, data, &mut sieve) {
            response::send_event(client, &event)?;
        }
        round.absorb(serde_json::from_value(value)?);
    }

    let tail = sieve.finish();
    if !tail.is_empty() {
        round.content.push_str(&tail);
        response::send_event(
            client,
            &json!({ "choices": [{ "delta": { "content": tail } }] }).to_string(),
        )?;
    }
    for (number, call) in sieve.take_calls().into_iter().enumerate() {
        round.calls.push(ToolCall {
            id: format!("text-{number}"),
            name: call.name,
            arguments: call.arguments,
        });
    }
    Ok(round)
}

/// Rewrites one chunk so only the visible part of its text is forwarded, and
/// returns the event to send. A chunk with no text, or with text that holds no
/// tool-call tag, is forwarded unchanged.
fn forward(value: &mut Value, data: &str, sieve: &mut Sieve) -> Option<String> {
    let Some(text) = value
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Some(data.to_string());
    };

    let was_open = sieve.is_open();
    let visible = sieve.push(&text);
    if !was_open && visible == text {
        return Some(data.to_string());
    }
    if let Some(slot) = value.pointer_mut("/choices/0/delta/content") {
        *slot = Value::String(visible.clone());
    }
    if visible.is_empty() && !carries_more(value) {
        return None;
    }
    Some(value.to_string())
}

/// Whether a chunk carries something besides its text: more tool-call pieces,
/// reasoning, or the finish reason.
fn carries_more(value: &Value) -> bool {
    let delta = value
        .pointer("/choices/0/delta")
        .and_then(Value::as_object)
        .is_some_and(|object| object.keys().any(|key| key != "content"));
    let finish = value
        .pointer("/choices/0/finish_reason")
        .is_some_and(|reason| !reason.is_null());
    delta || finish
}

impl Round {
    /// Adds the text and tool-call pieces of one streamed chunk.
    fn absorb(&mut self, chunk: Chunk) {
        let Some(Choice { delta }) = chunk.choices.into_iter().next() else {
            return;
        };

        if let Some(text) = delta.content {
            self.content.push_str(&text);
        }

        for piece in delta.tool_calls.unwrap_or_default() {
            if piece.index >= MAX_CALLS {
                continue;
            }

            if self.calls.len() <= piece.index {
                self.calls.resize_with(piece.index + 1, ToolCall::default);
            }
            self.calls[piece.index].extend(piece);
        }
    }

    /// The assistant message that asked for this round's tool calls.
    fn as_message(&self) -> Added<'_> {
        let tool_calls = self
            .calls
            .iter()
            .map(|call| CallRecord {
                id: &call.id,
                kind: "function",
                function: FunctionRecord {
                    name: &call.name,
                    arguments: &call.arguments,
                },
            })
            .collect();
        Added::Assistant {
            content: &self.content,
            tool_calls,
        }
    }
}

impl ToolCall {
    /// Appends one streamed piece.
    fn extend(&mut self, piece: CallPiece) {
        self.id.push_str(&piece.id.unwrap_or_default());

        if let Some(function) = piece.function {
            self.name.push_str(&function.name.unwrap_or_default());
            self.arguments
                .push_str(&function.arguments.unwrap_or_default());
        }
    }

    /// The arguments of this call, parsed.
    fn arguments<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_str(&self.arguments).ok()
    }

    /// The searches one call asks for: the `query`, plus every `queries` entry,
    /// each widened by `kind`, capped at [`MAX_QUERIES_PER_CALL`] and without
    /// repeats. `None` when no query is given. An unknown `time_range` or `kind`
    /// is treated as absent.
    fn search_plan(&self) -> Option<SearchPlan> {
        let arguments: SearchArguments = self.arguments()?;

        let mut base: Vec<String> = Vec::new();
        for query in arguments.query.as_deref().into_iter().chain(
            arguments
                .queries
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(String::as_str),
        ) {
            let query = query.trim();
            if !query.is_empty() && !base.iter().any(|seen| seen == query) {
                base.push(query.to_string());
            }
        }
        if base.is_empty() {
            return None;
        }

        let kind = Kind::of(arguments.kind.as_deref());
        let mut queries: Vec<String> = Vec::new();
        for query in base.iter().flat_map(|query| kind.widen(query)) {
            if queries.len() == MAX_QUERIES_PER_CALL {
                break;
            }
            if !queries.contains(&query) {
                queries.push(query);
            }
        }
        Some(SearchPlan {
            queries,
            range: arguments.time_range.as_deref().and_then(TimeRange::of),
        })
    }

    /// The `url` of a fetch, trimmed; `None` when missing or empty.
    fn url_argument(&self) -> Option<String> {
        let arguments: FetchArguments = self.arguments()?;
        let url = arguments.url.trim();
        (!url.is_empty()).then(|| url.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, FakeWeb, event_stream, json_response};

    fn upstreams(model: &FakeServer, search: &FakeServer) -> Upstreams {
        upstreams_with(model, search, FakeWeb::default())
    }

    fn upstreams_with(model: &FakeServer, search: &FakeServer, web: FakeWeb) -> Upstreams {
        Upstreams {
            model: Endpoint::new(model.address(), None),
            engines: Vec::new(),
            search: Endpoint::new(search.address(), None),
            web: Box::new(web),
        }
    }

    fn idle() -> Outcome<FakeServer> {
        FakeServer::start(Vec::new())
    }

    fn events(client: &[u8]) -> Vec<String> {
        String::from_utf8_lossy(client)
            .lines()
            .filter_map(|line| line.strip_prefix("data: ").map(ToString::to_string))
            .collect()
    }

    fn chunk(json: &str) -> Outcome<Chunk> {
        Ok(serde_json::from_str(json)?)
    }

    fn call_event(name: &str, arguments: &str) -> String {
        call_event_at(0, name, arguments)
    }

    fn call_event_at(index: usize, name: &str, arguments: &str) -> String {
        json!({"choices":[{"delta":{"tool_calls":[{"index":index,"id":format!("c{index}"),"function":{"name":name,"arguments":arguments}}]}}]})
            .to_string()
    }

    #[test]
    fn the_mode_is_a_truth_table() {
        assert_eq!(Mode::of(true, true), Mode::Search);
        assert_eq!(Mode::of(true, false), Mode::Search);
        assert_eq!(Mode::of(false, true), Mode::Stream);
        assert_eq!(Mode::of(false, false), Mode::Relay);
    }

    #[test]
    fn tools_are_found_by_name() {
        assert_eq!(Tool::named("web_search"), Some(Tool::WebSearch));
        assert_eq!(
            Tool::named("fetch_page_content_recursive"),
            Some(Tool::FetchPage)
        );
        assert_eq!(Tool::named("fetch_page"), None);
        assert_eq!(Tool::ALL.len(), 2);
        assert_eq!(
            Tool::WebSearch.definition()["function"]["name"],
            "web_search"
        );
        assert_eq!(
            Tool::FetchPage.definition()["function"]["name"],
            "fetch_page_content_recursive"
        );
        assert_eq!(
            Tool::WebSearch.definition()["function"]["parameters"]["properties"]["time_range"]["enum"]
                [0],
            "day"
        );
    }

    #[test]
    fn a_search_request_gains_a_system_hint() {
        let mut fields = Fields::new();
        fields.insert(
            MESSAGES.to_string(),
            json!([{ "role": "user", "content": "news?" }]),
        );
        nudge_to_search(&mut fields);
        assert_eq!(
            fields[MESSAGES],
            json!([{ "role": "system", "content": SEARCH_HINT }, { "role": "user", "content": "news?" }])
        );
    }

    #[test]
    fn the_hint_merges_into_an_existing_system_message() {
        let mut fields = Fields::new();
        fields.insert(MESSAGES.to_string(), json!([{ "role": "system", "content": "Be brief." }, { "role": "user", "content": "hi" }]));
        nudge_to_search(&mut fields);
        assert_eq!(
            fields[MESSAGES][0]["content"],
            json!(format!("Be brief.\n\n{SEARCH_HINT}"))
        );
        assert_eq!(fields[MESSAGES].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn the_hint_is_skipped_without_a_message_list() {
        let mut fields = Fields::new();
        fields.insert(STREAM.to_string(), Value::Bool(true));
        nudge_to_search(&mut fields);
        assert!(!fields.contains_key(MESSAGES));
    }

    #[test]
    fn taking_the_hint_back_leaves_the_rest_of_the_system_line() {
        let content = format!("Be brief.\n\n{SEARCH_HINT}\n\nBe honest.");
        let system = |fields: &Fields| fields[MESSAGES][0]["content"].clone();
        let mut fields = Fields::new();
        fields.insert(
            MESSAGES.to_string(),
            json!([{ "role": "system", "content": content }, { "role": "user", "content": "hi" }]),
        );
        retract_system(&mut fields, SEARCH_HINT);
        assert_eq!(system(&fields), json!("Be brief.\n\nBe honest."));
        assert_eq!(fields[MESSAGES].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn taking_back_a_hint_that_is_the_whole_system_line_leaves_it_empty() {
        let mut fields = Fields::new();
        fields.insert(
            MESSAGES.to_string(),
            json!([{ "role": "system", "content": SEARCH_HINT }]),
        );
        retract_system(&mut fields, SEARCH_HINT);
        assert_eq!(fields[MESSAGES][0]["content"], json!(""));
    }

    #[test]
    fn taking_back_a_hint_that_was_never_added_changes_nothing() {
        let messages = json!([{ "role": "system", "content": "Be brief." }]);
        let mut fields = Fields::new();
        fields.insert(MESSAGES.to_string(), messages.clone());
        retract_system(&mut fields, SEARCH_HINT);
        assert_eq!(fields[MESSAGES], messages);
    }

    #[test]
    fn a_system_message_that_is_not_text_is_left_alone() {
        let messages = json!([{ "role": "system", "content": [{ "type": "text", "text": "hi" }] }]);
        let mut fields = Fields::new();
        fields.insert(MESSAGES.to_string(), messages.clone());
        nudge_to_search(&mut fields);
        assert_eq!(fields[MESSAGES], messages);
    }

    #[test]
    fn assembles_a_tool_call_streamed_in_pieces() -> Outcome {
        let mut round = Round::default();
        round.absorb(chunk(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a1","function":{"name":"web_search","arguments":"{\"que"}}]}}]}"#)?);
        round.absorb(chunk(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ry\":\"rust\"}"}}]}}]}"#)?);
        round.absorb(chunk(
            r#"{"choices":[{"delta":{"content":"ok","tool_calls":null}}]}"#,
        )?);
        round.absorb(chunk(r#"{"choices":[],"timings":{}}"#)?);
        assert_eq!(round.content, "ok");
        assert_eq!(round.calls.len(), 1);
        assert_eq!(
            round.calls[0].search_plan(),
            Some(SearchPlan {
                queries: vec!["rust".to_string()],
                range: None
            })
        );
        Ok(())
    }

    #[test]
    fn ignores_tool_calls_past_the_limit() -> Outcome {
        let mut round = Round::default();
        round.absorb(chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1000000,"id":"x"}]}}]}"#,
        )?);
        assert_eq!(round.calls, []);
        Ok(())
    }

    #[test]
    fn a_search_plan_holds_the_queries_and_the_recency() {
        let plan = |arguments: &str| {
            ToolCall {
                arguments: arguments.to_string(),
                ..ToolCall::default()
            }
            .search_plan()
        };

        assert_eq!(
            plan(r#"{"query":"  rust  "}"#),
            Some(SearchPlan {
                queries: vec!["rust".to_string()],
                range: None
            })
        );
        assert_eq!(
            plan(r#"{"query":"jobs","time_range":"week"}"#),
            Some(SearchPlan {
                queries: vec!["jobs".to_string()],
                range: Some(TimeRange::Week)
            })
        );
        assert_eq!(
            plan(r#"{"query":"jobs","time_range":"forever"}"#),
            Some(SearchPlan {
                queries: vec!["jobs".to_string()],
                range: None
            })
        );
        assert_eq!(
            plan(r#"{"query":"rust","queries":["rust","tokio","  ","axum"]}"#),
            Some(SearchPlan {
                queries: vec!["rust".to_string(), "tokio".to_string(), "axum".to_string()],
                range: None
            })
        );
        assert_eq!(
            plan(r#"{"query":"rust engineer","kind":"jobs"}"#),
            Some(SearchPlan {
                queries: vec![
                    "rust engineer".to_string(),
                    "rust engineer job posting".to_string(),
                    "rust engineer careers".to_string(),
                    "rust engineer linkedin jobs".to_string(),
                ],
                range: None
            })
        );
        assert_eq!(
            plan(r#"{"query":"synthires","kind":"people"}"#).map(|plan| plan.queries.len()),
            Some(MAX_QUERIES_PER_CALL)
        );
        assert_eq!(
            plan(r#"{"queries":["only this"]}"#),
            Some(SearchPlan {
                queries: vec!["only this".to_string()],
                range: None
            })
        );
        assert_eq!(
            plan(r#"{"queries":["a","b"],"kind":"people"}"#).map(|plan| plan.queries.len()),
            Some(MAX_QUERIES_PER_CALL)
        );
        assert_eq!(plan(r#"{"query":"  "}"#), None);
        assert_eq!(plan(r#"{"q":"rust"}"#), None);
        assert_eq!(plan("not json"), None);
    }

    #[test]
    fn a_tool_call_must_carry_its_argument() {
        let call = |arguments: &str| ToolCall {
            arguments: arguments.to_string(),
            ..ToolCall::default()
        };
        assert_eq!(
            call(r#"{"url":" https://example.com/ "}"#)
                .url_argument()
                .as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(call(r#"{"url":"  "}"#).url_argument(), None);
        assert_eq!(call("[]").url_argument(), None);
    }

    #[test]
    fn a_search_call_with_several_queries_fans_out() -> Outcome {
        let call = call_event(
            "web_search",
            r#"{"query":"rust jobs","queries":["tokio jobs"]}"#,
        );
        let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
        let search = FakeServer::start(vec![
            json_response(
                r#"{"results":[{"title":"A","url":"https://a"},{"title":"B","url":"https://b"}]}"#,
            ),
            json_response(
                r#"{"results":[{"title":"B again","url":"https://b"},{"title":"C","url":"https://c"}]}"#,
            ),
        ])?;
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"messages":[{"role":"user","content":"jobs"}],"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;

        let asked = search.requests()?;
        assert_eq!(asked.len(), 2);
        assert!(asked[0].contains("q=rust+jobs"));
        assert!(asked[1].contains("q=tokio+jobs"));

        let seen = model.requests()?;
        assert!(seen[1].contains("[1] A"), "{}", seen[1]);
        assert!(seen[1].contains("[2] B"), "{}", seen[1]);
        assert!(seen[1].contains("[3] C"), "{}", seen[1]);
        assert!(seen[1].contains("Searches used 2 of 16"), "{}", seen[1]);

        let sent = events(&client);
        assert_eq!(
            sent.iter()
                .filter(|event| event.contains(r#""search""#))
                .count(),
            2
        );
        Ok(())
    }

    #[test]
    fn a_people_search_is_widened_towards_recruiters() -> Outcome {
        let call = call_event(
            "web_search",
            r#"{"query":"synthires rust","kind":"people"}"#,
        );
        let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
        let search = FakeServer::start(vec![
            json_response(r#"{"results":[]}"#);
            MAX_QUERIES_PER_CALL
        ])?;
        answer(
            &mut Vec::new(),
            br#"{"messages":[{"role":"user","content":"who hires"}],"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;

        let asked = search.requests()?;
        assert_eq!(asked.len(), MAX_QUERIES_PER_CALL);
        assert!(asked[0].contains("q=synthires+rust&"), "{}", asked[0]);
        assert!(
            asked[1].contains("q=synthires+rust+recruiter"),
            "{}",
            asked[1]
        );
        assert!(asked[2].contains("hiring+manager"), "{}", asked[2]);
        assert!(asked[3].contains("we+are+hiring"), "{}", asked[3]);
        Ok(())
    }

    #[test]
    fn the_search_budget_stops_the_seventeenth_query() -> Outcome {
        let wide = |call: usize| {
            call_event_at(
                call,
                "web_search",
                &format!(r#"{{"queries":["q{call}a","q{call}b","q{call}c","q{call}d"]}}"#),
            )
        };
        let stream = event_stream(&[&wide(0), &wide(1), &wide(2), &wide(3), &wide(4)]);
        let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
        let model = FakeServer::start(vec![stream, event_stream(&[reply])])?;
        let search = FakeServer::start(vec![json_response(r#"{"results":[]}"#); MAX_SEARCHES])?;
        answer(
            &mut Vec::new(),
            br#"{"messages":[{"role":"user","content":"hunt"}],"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;

        assert_eq!(search.requests()?.len(), MAX_SEARCHES);
        let seen = model.requests()?;
        assert_eq!(seen.len(), 2);
        assert!(seen[1].contains("budget is spent"), "{}", seen[1]);
        assert!(seen[1].contains("Searches used 16 of 16"), "{}", seen[1]);
        Ok(())
    }

    #[test]
    fn the_answer_round_is_given_the_source_numbers() -> Outcome {
        let calls: Vec<String> = (1..=MAX_ROUNDS)
            .map(|round| call_event("web_search", &format!(r#"{{"query":"q{round}"}}"#)))
            .collect();
        let mut responses: Vec<String> = calls
            .iter()
            .map(|call| event_stream(&[call.as_str()]))
            .collect();
        responses.push(event_stream(&[
            r#"{"choices":[{"delta":{"content":"done"}}]}"#,
        ]));
        let model = FakeServer::start(responses)?;
        let search = FakeServer::start(vec![
            json_response(
                r#"{"results":[{"title":"Rust","url":"https://r"}]}"#
            );
            MAX_ROUNDS
        ])?;
        answer(
            &mut Vec::new(),
            br#"{"messages":[{"role":"user","content":"news"}],"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;

        let seen = model.requests()?;
        assert_eq!(seen.len(), MAX_ROUNDS + 1);
        assert!(
            seen[MAX_ROUNDS].contains("Sources found, with the numbers to cite"),
            "{}",
            seen[MAX_ROUNDS]
        );
        assert!(
            seen[MAX_ROUNDS].contains("[1] Rust — https://r"),
            "{}",
            seen[MAX_ROUNDS]
        );
        assert!(
            !seen[MAX_ROUNDS].contains(SEARCH_HINT),
            "the search hint must be gone by the answer rounds"
        );
        Ok(())
    }

    #[test]
    fn added_messages_have_openais_shape() -> Outcome {
        let round = Round {
            content: "thinking".to_string(),
            calls: vec![ToolCall {
                id: "c1".to_string(),
                name: "web_search".to_string(),
                arguments: "{}".to_string(),
            }],
        };
        assert_eq!(
            serde_json::to_value(round.as_message())?,
            json!({"role":"assistant","content":"thinking","tool_calls":[{"id":"c1","type":"function","function":{"name":"web_search","arguments":"{}"}}]})
        );
        assert_eq!(
            serde_json::to_value(Added::Tool {
                tool_call_id: "c1",
                content: "[1] x"
            })?,
            json!({"role":"tool","tool_call_id":"c1","content":"[1] x"})
        );
        Ok(())
    }

    #[test]
    fn relays_a_request_that_does_not_stream() -> Outcome {
        let (model, search) = (
            FakeServer::start(vec![json_response(r#"{"choices":[]}"#)])?,
            idle()?,
        );
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"messages":[]}"#,
            &upstreams(&model, &search),
        )?;
        assert!(String::from_utf8_lossy(&client).ends_with(r#"{"choices":[]}"#));
        assert!(model.requests()?[0].ends_with(r#"{"messages":[]}"#));
        Ok(())
    }

    #[test]
    fn streams_tokens_and_ends_with_done() -> Outcome {
        let token = r#"{"choices":[{"delta":{"content":"hi"}}]}"#;
        let (model, search) = (FakeServer::start(vec![event_stream(&[token])])?, idle()?);
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"messages":[],"stream":true}"#,
            &upstreams(&model, &search),
        )?;
        assert_eq!(events(&client), [token, DONE]);
        assert!(!model.requests()?[0].contains(SEARCH_HINT));
        Ok(())
    }

    #[test]
    fn a_web_search_request_tells_the_model_to_search() -> Outcome {
        let call = call_event("web_search", r#"{"query":"rust"}"#);
        let reply = r#"{"choices":[{"delta":{"content":"ok"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
        let search = FakeServer::start(vec![json_response(r#"{"results":[]}"#)])?;
        let request = br#"{"messages":[{"role":"user","content":"news?"}],"thor_web_search":true}"#;
        answer(&mut Vec::new(), request, &upstreams(&model, &search))?;
        let seen = model.requests()?;
        assert!(seen[0].contains(SEARCH_HINT) && seen[0].contains(r#""role":"system""#));
        Ok(())
    }

    #[test]
    fn searches_then_answers() -> Outcome {
        let call = call_event("web_search", r#"{"query":"rust"}"#);
        let reply = r#"{"choices":[{"delta":{"content":"Rust 1.99"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
        let search = FakeServer::start(vec![json_response(
            r#"{"results":[{"title":"Rust","url":"https://r","content":"new"}]}"#,
        )])?;
        let mut client = Vec::new();
        let request = br#"{"messages":[{"role":"user","content":"news?"}],"thor_web_search":true}"#;
        answer(&mut client, request, &upstreams(&model, &search))?;
        let seen = model.requests()?;
        assert!(seen[0].contains(r#""tools""#) && seen[0].contains(r#""stream":true"#));
        assert!(seen[1].contains(r#""role":"tool""#) && seen[1].contains("[1] Rust"));
        let sent = events(&client);
        assert_eq!(sent.first(), Some(&call));
        assert!(sent.contains(&r#"{"thor":{"search":{"query":"rust","results":[{"title":"Rust","url":"https://r"}]}}}"#.to_string()));
        assert_eq!(sent.last().map(String::as_str), Some(DONE));
        Ok(())
    }

    #[test]
    fn every_tool_round_has_tools_and_one_last_round_does_not() -> Outcome {
        let call = call_event("other", "{}");
        let reply = r#"{"choices":[{"delta":{"content":"final answer"}}]}"#;
        let mut responses = vec![event_stream(&[&call]); MAX_ROUNDS];
        responses.push(event_stream(&[reply]));
        let model = FakeServer::start(responses)?;
        answer(
            &mut Vec::new(),
            br#"{"messages":[],"thor_web_search":true}"#,
            &upstreams(&model, &idle()?),
        )?;
        let seen = model.requests()?;
        assert_eq!(seen.len(), MAX_ROUNDS + 1);
        assert!(
            seen[..MAX_ROUNDS]
                .iter()
                .all(|request| request.contains(r#""tools""#))
        );
        assert!(!seen[MAX_ROUNDS].contains(r#""tools""#));
        assert!(seen[MAX_ROUNDS].contains(ANSWER_NUDGE));
        assert!(seen[MAX_ROUNDS].contains(ANSWER_ASK));
        assert!(seen[1].contains("Unknown tool other."));
        assert!(seen[2].contains("already ran"), "{}", seen[2]);
        Ok(())
    }

    #[test]
    fn a_call_written_as_text_runs_and_is_not_shown() -> Outcome {
        let text = "Let me look.\n<tool_call>\n<function=web_search>\n<parameter=query>\nrust jobs\n</parameter>\n</function>\n</tool_call>\nDone.";
        let call = json!({ "choices": [{ "delta": { "content": text } }] }).to_string();
        let reply = r#"{"choices":[{"delta":{"content":"Here is the answer"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
        let search = FakeServer::start(vec![json_response(
            r#"{"results":[{"title":"A job","url":"https://jobs.example/1","content":"hiring"}]}"#,
        )])?;
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"messages":[{"role":"user","content":"jobs?"}],"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;
        let seen = model.requests()?;
        assert!(seen[1].contains("[1] A job"), "{}", seen[1]);
        assert!(search.requests()?[0].contains("q=rust+jobs"));
        let sent = events(&client);
        assert!(
            !sent.iter().any(|event| event.contains("<tool_call")),
            "{sent:?}"
        );
        assert!(
            sent.iter().any(|event| event.contains("Let me look.")),
            "{sent:?}"
        );
        assert!(
            sent.iter()
                .any(|event| event.contains("Here is the answer")),
            "{sent:?}"
        );
        Ok(())
    }

    #[test]
    fn a_tag_the_model_leaves_open_is_flushed_at_the_end_of_the_round() -> Outcome {
        let call = call_event("web_search", r#"{"query":"rust"}"#);
        let partial =
            json!({ "choices": [{ "delta": { "content": "almost <tool" } }] }).to_string();
        let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[&partial])])?;
        let search = FakeServer::start(vec![json_response(r#"{"results":[]}"#)])?;
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"messages":[{"role":"user","content":"news"}],"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;
        let sent = events(&client);
        assert!(
            sent.iter().any(|event| event.contains("almost ")),
            "{sent:?}"
        );
        assert!(sent.iter().any(|event| event.contains("<tool")), "{sent:?}");
        Ok(())
    }

    #[test]
    fn a_query_that_fails_is_reported_and_the_others_still_run() -> Outcome {
        let call = call_event("web_search", r#"{"query":"good","queries":["bad"]}"#);
        let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
        let search = FakeServer::start(vec![json_response(
            r#"{"results":[{"title":"Good","url":"https://good"}]}"#,
        )])?;
        answer(
            &mut Vec::new(),
            br#"{"messages":[{"role":"user","content":"news"}],"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;
        let seen = model.requests()?;
        assert!(seen[1].contains("bad"), "{}", seen[1]);
        assert!(seen[1].contains("failed"), "{}", seen[1]);
        assert!(seen[1].contains("[1] Good"), "{}", seen[1]);
        assert!(seen[1].contains("Searches used 2 of 16"), "{}", seen[1]);
        Ok(())
    }

    #[test]
    fn engines_that_did_not_answer_reach_the_model() -> Outcome {
        let call = call_event("web_search", r#"{"query":"rust"}"#);
        let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
        let search = FakeServer::start(vec![json_response(
            r#"{"results":[],"unresponsive_engines":[["duckduckgo","CAPTCHA"],["brave",null]]}"#,
        )])?;
        answer(
            &mut Vec::new(),
            br#"{"messages":[{"role":"user","content":"news"}],"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;
        let seen = model.requests()?;
        assert!(
            seen[1].contains("Engines that did not answer: duckduckgo (CAPTCHA), brave."),
            "{}",
            seen[1]
        );
        Ok(())
    }

    #[test]
    fn the_page_budget_stops_after_twelve_pages() -> Outcome {
        let links = "<a href=\"/a\">a</a><a href=\"/b\">b</a><a href=\"/c\">c</a><a href=\"/d\">d</a><a href=\"/e\">e</a><a href=\"/f\">f</a>";
        let page = |name: &str| fetched("text/html", &format!("<title>{name}</title>{links}"));
        let web = FakeWeb::new(vec![
            ("https://docs.example/a", page("A")),
            ("https://docs.example/b", page("B")),
            ("https://docs.example/c", page("C")),
            ("https://docs.example/d", page("D")),
            ("https://docs.example/e", page("E")),
            ("https://docs.example/f", page("F")),
        ]);

        let fetch_call = |url: &str| {
            call_event(
                "fetch_page_content_recursive",
                &format!(r#"{{"url":"{url}"}}"#),
            )
        };
        let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
        let mut responses = vec![
            event_stream(&[&fetch_call("https://docs.example/a")]),
            event_stream(&[&fetch_call("https://docs.example/b")]),
            event_stream(&[&fetch_call("https://docs.example/c")]),
        ];
        responses.push(event_stream(&[reply]));
        let model = FakeServer::start(responses)?;
        answer(
            &mut Vec::new(),
            br#"{"messages":[{"role":"user","content":"read https://docs.example/a https://docs.example/b https://docs.example/c"}],"thor_web_search":true}"#,
            &upstreams_with(&model, &idle()?, web),
        )?;
        let seen = model.requests()?;
        assert!(seen[3].contains("page budget is spent"), "{}", seen[3]);
        assert!(seen[3].contains("(12 pages)"), "{}", seen[3]);
        Ok(())
    }

    #[test]
    fn a_call_on_the_answer_round_is_dropped_and_asked_again() -> Outcome {
        let call = call_event("other", "{}");
        let leaked = json!({ "choices": [{ "delta": { "content": "<tool_call><function=web_search><parameter=query>x</parameter></function></tool_call>" } }] }).to_string();
        let reply = r#"{"choices":[{"delta":{"content":"the answer"}}]}"#;
        let mut responses = vec![event_stream(&[&call]); MAX_ROUNDS];
        responses.push(event_stream(&[&leaked]));
        responses.push(event_stream(&[reply]));
        let model = FakeServer::start(responses)?;
        let search = idle()?;
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"messages":[],"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;
        let seen = model.requests()?;
        assert_eq!(seen.len(), MAX_ROUNDS + 2);
        assert!(
            seen[MAX_ROUNDS + 1].contains(ANSWER_AGAIN),
            "{}",
            seen[MAX_ROUNDS + 1]
        );
        assert!(
            search.requests()?.is_empty(),
            "a call written with the tools closed must not run"
        );
        let sent = events(&client);
        assert!(
            !sent.iter().any(|event| event.contains("<tool_call")),
            "{sent:?}"
        );
        assert!(
            sent.iter().any(|event| event.contains("the answer")),
            "{sent:?}"
        );
        Ok(())
    }

    #[test]
    fn text_beside_a_call_on_the_answer_round_is_the_answer() -> Outcome {
        let call = call_event("other", "{}");
        let mixed = json!({ "choices": [{ "delta": { "content": "Rust 1.99 is out. <tool_call><function=web_search><parameter=query>x</parameter></function></tool_call>" } }] }).to_string();
        let mut responses = vec![event_stream(&[&call]); MAX_ROUNDS];
        responses.push(event_stream(&[&mixed]));
        let model = FakeServer::start(responses)?;
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"messages":[],"thor_web_search":true}"#,
            &upstreams(&model, &idle()?),
        )?;
        let seen = model.requests()?;
        assert_eq!(seen.len(), MAX_ROUNDS + 1);
        let sent = events(&client);
        assert!(
            sent.iter().any(|event| event.contains("Rust 1.99 is out.")),
            "{sent:?}"
        );
        assert!(
            !sent.iter().any(|event| event.contains("<tool_call")),
            "{sent:?}"
        );
        Ok(())
    }

    #[test]
    fn the_fallback_is_sent_when_no_round_writes_an_answer() -> Outcome {
        let call = call_event("other", "{}");
        let empty = r#"{"choices":[{"delta":{}}]}"#;
        let mut responses = vec![event_stream(&[&call]); MAX_ROUNDS];
        responses.extend((0..ANSWER_ROUNDS).map(|_| event_stream(&[empty])));
        let model = FakeServer::start(responses)?;
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"messages":[],"thor_web_search":true}"#,
            &upstreams(&model, &idle()?),
        )?;
        let sent = events(&client);
        assert!(
            sent.iter()
                .any(|event| event.contains("no round wrote an answer")),
            "{sent:?}"
        );
        Ok(())
    }

    #[test]
    fn the_fallback_names_the_sources_when_no_round_writes_one() -> Outcome {
        let call = call_event("web_search", r#"{"query":"nvidia jobs"}"#);
        let empty = r#"{"choices":[{"delta":{}}]}"#;
        let mut responses = vec![event_stream(&[&call]); MAX_ROUNDS];
        responses.extend((0..ANSWER_ROUNDS).map(|_| event_stream(&[empty])));
        let model = FakeServer::start(responses)?;
        let search = FakeServer::start(vec![json_response(
            r#"{"results":[{"title":"A job","url":"https://jobs.example/1"}]}"#,
        )])?;
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"messages":[],"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;
        let sent = events(&client);
        assert!(
            sent.iter()
                .any(|event| event.contains("[1] A job — https://jobs.example/1")),
            "{sent:?}"
        );
        Ok(())
    }

    #[test]
    fn a_model_error_becomes_an_error_event() -> Outcome {
        let (model, search) = (
            FakeServer::start(vec!["HTTP/1.1 500 Oops\r\n\r\nbroken".to_string()])?,
            idle()?,
        );
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"messages":[],"stream":true}"#,
            &upstreams(&model, &search),
        )?;
        let sent = events(&client);
        assert_eq!(
            sent,
            [
                r#"{"thor":{"error":"upstream: model server returned 500: broken"}}"#,
                DONE
            ]
        );
        Ok(())
    }

    #[test]
    fn rejects_a_body_that_is_not_an_object() -> Outcome {
        let (model, search) = (idle()?, idle()?);
        let outcome = answer(&mut Vec::new(), b"[]", &upstreams(&model, &search));
        assert!(
            outcome.is_err_and(
                |error| error.to_string() == "bad request: the body must be a JSON object"
            )
        );
        Ok(())
    }

    #[test]
    fn the_first_round_requires_a_tool() {
        let mut fields = Fields::new();
        require_tool(&mut fields, true);
        assert_eq!(fields[TOOL_CHOICE], json!("required"));
        require_tool(&mut fields, false);
        assert!(!fields.contains_key(TOOL_CHOICE));
    }

    #[test]
    fn the_user_text_is_only_the_users_messages() {
        let messages = json!({
            "messages": [
                { "role": "system", "content": "hint" },
                { "role": "user", "content": "read https://user.example/doc" },
                { "role": "assistant", "content": "sure" },
                { "role": "user", "content": [{ "type": "text" }] }
            ]
        });
        let fields = messages.as_object().cloned().unwrap_or_default();
        assert_eq!(user_text(&fields), "read https://user.example/doc");
        assert_eq!(user_text(&Fields::new()), "");
    }

    fn fetched(content_type: &str, body: &str) -> crate::http::Fetched {
        crate::http::Fetched {
            status: 200,
            content_type: content_type.to_string(),
            location: None,
            body: body.to_string(),
        }
    }

    #[test]
    fn searches_then_answers_with_a_forced_first_round_and_recency() -> Outcome {
        let call = call_event("web_search", r#"{"query":"rust","time_range":"day"}"#);
        let reply = r#"{"choices":[{"delta":{"content":"Rust 1.99"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
        let search = FakeServer::start(vec![json_response(
            r#"{"results":[{"title":"Rust","url":"https://r","content":"new","publishedDate":"2026-10-06"}]}"#,
        )])?;
        let mut client = Vec::new();
        let request = br#"{"messages":[{"role":"user","content":"news?"}],"thor_web_search":true}"#;
        answer(&mut client, request, &upstreams(&model, &search))?;
        let seen = model.requests()?;
        assert!(seen[0].contains(r#""tools""#) && seen[0].contains(r#""stream":true"#));
        assert!(
            seen[0].contains(r#""tool_choice":"required""#),
            "{}",
            seen[0]
        );
        assert!(!seen[1].contains(r#""tool_choice""#));
        assert!(seen[1].contains(r#""role":"tool""#) && seen[1].contains("[1] Rust"));
        assert!(seen[1].contains("published: 2026-10-06"), "{}", seen[1]);
        assert!(search.requests()?[0].contains("time_range=day"));
        let sent = events(&client);
        assert_eq!(sent.first(), Some(&call));
        assert!(sent.contains(&r#"{"thor":{"search":{"query":"rust","results":[{"title":"Rust","url":"https://r"}]}}}"#.to_string()));
        assert_eq!(sent.last().map(String::as_str), Some(DONE));
        Ok(())
    }

    #[test]
    fn reads_a_cited_page_with_the_fetch_tool() -> Outcome {
        let search_call = call_event("web_search", r#"{"query":"rust"}"#);
        let fetch_call = call_event(
            "fetch_page_content_recursive",
            r#"{"url":"https://docs.example/guide"}"#,
        );
        let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
        let model = FakeServer::start(vec![
            event_stream(&[&search_call]),
            event_stream(&[&fetch_call]),
            event_stream(&[reply]),
        ])?;
        let search = FakeServer::start(vec![json_response(
            r#"{"results":[{"title":"Guide","url":"https://docs.example/guide","content":"see"}]}"#,
        )])?;
        let web = FakeWeb::new(vec![(
            "https://docs.example/guide",
            fetched("text/html", "<title>Guide</title><p>the answer</p>"),
        )]);
        let mut client = Vec::new();
        let request = br#"{"messages":[{"role":"user","content":"how?"}],"thor_web_search":true}"#;
        answer(&mut client, request, &upstreams_with(&model, &search, web))?;
        let seen = model.requests()?;
        assert!(seen[2].contains("the answer"), "{}", seen[2]);
        let sent = events(&client);
        assert!(
            sent.contains(
                &json!({ "thor": { "read": { "url": "https://docs.example/guide", "title": "Guide" } } })
                    .to_string()
            ),
            "{sent:?}"
        );
        Ok(())
    }

    #[test]
    fn an_address_the_user_wrote_may_be_read() -> Outcome {
        let fetch_call = call_event(
            "fetch_page_content_recursive",
            r#"{"url":"https://user.example/doc"}"#,
        );
        let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&fetch_call]), event_stream(&[reply])])?;
        let web = FakeWeb::new(vec![(
            "https://user.example/doc",
            fetched("text/plain", "hello from the page"),
        )]);
        answer(
            &mut Vec::new(),
            br#"{"messages":[{"role":"user","content":"read https://user.example/doc"}],"thor_web_search":true}"#,
            &upstreams_with(&model, &idle()?, web),
        )?;
        assert!(model.requests()?[1].contains("hello from the page"));
        Ok(())
    }

    #[test]
    fn an_address_that_was_not_seen_is_refused() -> Outcome {
        let fetch_call = call_event(
            "fetch_page_content_recursive",
            r#"{"url":"https://evil.example/"}"#,
        );
        let reply = r#"{"choices":[{"delta":{"content":"ok"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&fetch_call]), event_stream(&[reply])])?;
        let web = FakeWeb::new(vec![(
            "https://evil.example/",
            fetched("text/html", "secret"),
        )]);
        answer(
            &mut Vec::new(),
            br#"{"messages":[{"role":"user","content":"hi"}],"thor_web_search":true}"#,
            &upstreams_with(&model, &idle()?, web),
        )?;
        let seen = model.requests()?;
        assert!(
            seen[1].contains("not in this answer's search results"),
            "{}",
            seen[1]
        );
        assert!(!seen[1].contains("secret"));
        Ok(())
    }

    #[test]
    fn a_page_that_cannot_be_read_is_reported() -> Outcome {
        let fetch_call = call_event(
            "fetch_page_content_recursive",
            r#"{"url":"https://gone.example/"}"#,
        );
        let reply = r#"{"choices":[{"delta":{"content":"ok"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&fetch_call]), event_stream(&[reply])])?;
        answer(
            &mut Vec::new(),
            br#"{"messages":[{"role":"user","content":"read https://gone.example/"}],"thor_web_search":true}"#,
            &upstreams(&model, &idle()?),
        )?;
        let seen = model.requests()?;
        assert!(seen[1].contains("Could not read the page"), "{}", seen[1]);
        Ok(())
    }

    #[test]
    fn a_fetch_without_an_address_or_with_a_bad_one_says_so() -> Outcome {
        let model = FakeServer::start(vec![
            event_stream(&[&call_event("fetch_page_content_recursive", "{}")]),
            event_stream(&[&call_event(
                "fetch_page_content_recursive",
                r#"{"url":"ftp://example.com"}"#,
            )]),
            event_stream(&[r#"{"choices":[{"delta":{"content":"ok"}}]}"#]),
        ])?;
        answer(
            &mut Vec::new(),
            br#"{"messages":[{"role":"user","content":"hi"}],"thor_web_search":true}"#,
            &upstreams(&model, &idle()?),
        )?;
        let seen = model.requests()?;
        assert!(
            seen[2].contains("The fetch needs an address."),
            "{}",
            seen[2]
        );
        assert!(
            seen[2].contains("refused: ftp://example.com: only https addresses may be read"),
            "{}",
            seen[2]
        );
        Ok(())
    }

    #[test]
    fn a_search_without_messages_is_reported() -> Outcome {
        let (model, search) = (
            FakeServer::start(vec![event_stream(&[&call_event("web_search", "{}")])])?,
            idle()?,
        );
        let mut client = Vec::new();
        answer(
            &mut client,
            br#"{"thor_web_search":true}"#,
            &upstreams(&model, &search),
        )?;
        assert!(
            events(&client)
                .iter()
                .any(|event| event.contains("messages must be a list"))
        );
        Ok(())
    }
}
