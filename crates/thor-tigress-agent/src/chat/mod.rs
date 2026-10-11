//! `/v1/chat/completions`: relayed as it is, streamed, or, with web search
//! on, run as a loop in which the model may call a tool before it answers.
//!
//! ```text
//!   client ── request ──► server ── stream ──► llama-server or an engine
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
//! [`crate::chat::tooltext`] takes it out of the answer and hands it to the loop.
//!
//! A request that brings its own tools comes from a coding tool running its
//! own loop, so the answer is copied back untouched, streamed or not. A
//! request for an Edge-LLM engine is first given the shape that engine takes;
//! see [`shape`].

mod shape;
pub mod tooltext;
mod wire;

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use self::wire::{Added, Round, Source, ThorEvent, ToolCall};
use crate::{
    web::{
        address::Url,
        fetch::{self, Allowed},
        http::Web,
        research::{Kind, Ledger, MAX_READS, MAX_SEARCHES},
        search::{self, SearchResult, TimeRange},
    },
    engines::{
        body::{self, Fields},
        upstream::{Endpoint, UpstreamResponse},
    },
    config::{Engine, Upstreams},
    error::{AgentError, Outcome},
    paths,
    server::response::{self, Client, DONE, EVENT_PREFIX},
    chat::tooltext::Sieve,
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

/// The most answer text one round may stream, in bytes. A model that falls into
/// a repeat loop would otherwise stream until its context fills, so a round is
/// cut here and the upstream connection is closed: every answer ends, whatever
/// the engine does.
const MAX_ROUND_CHARS: usize = 48 * 1024;

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

/// The most sources named in the last-resort note, which is all the page shows
/// when no round wrote a word.
const MAX_LISTED: usize = 10;

/// The request field asking for a streamed answer.
const STREAM: &str = "stream";

/// The request field listing the tools the model may call.
const TOOLS: &str = "tools";

/// The request field forcing or allowing a tool call.
const TOOL_CHOICE: &str = "tool_choice";

/// The `tool_choice` value that makes the model call a tool.
const REQUIRED: &str = "required";

/// The request field holding the conversation.
const MESSAGES: &str = "messages";

/// How a chat request is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Passed to the engine and its answer copied back as it is.
    Relay,
    /// Streamed to the client as events.
    Stream,
    /// Streamed, with tools available.
    Search,
}

impl Mode {
    /// The mode for a request: whether the page turned web search on, whether
    /// it asks for a stream, and whether it brings tools of its own.
    fn of(web_search: bool, streamed: bool, own_tools: bool) -> Mode {
        match (web_search, streamed, own_tools) {
            (true, _, _) => Mode::Search,
            (false, true, false) => Mode::Stream,
            (false, true, true) | (false, false, _) => Mode::Relay,
        }
    }
}

/// Whether the request lists tools for the model to call.
fn brings_tools(fields: &Fields) -> bool {
    fields
        .get(TOOLS)
        .and_then(Value::as_array)
        .is_some_and(|tools| !tools.is_empty())
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

    /// What the tool is for, as the model reads it.
    fn description(self) -> &'static str {
        match self {
            Tool::WebSearch => concat!(
                "Research the web. Returns titles, addresses, dates and short snippets, each ",
                "with a number to cite. `query` is the sub-question to search; `queries` ",
                "adds up to three more, so one call can fan out over the parts of a ",
                "question. `kind` widens the search: \"jobs\" looks for postings, ",
                "\"people\" for the recruiter and hiring manager behind one, whose posts ",
                "often mention a role first. `time_range` keeps results recent: day, ",
                "week, month or year."
            ),
            Tool::FetchPage => concat!(
                "Read a web page as plain text, following the page's own links up to two hops, ",
                "so the answer can use the page itself and not only a snippet. Only an https ",
                "address from this answer's search results, or one the user wrote, can be opened."
            ),
        }
    }

    /// The tool's arguments, as a JSON schema.
    fn parameters(self) -> Value {
        match self {
            Tool::WebSearch => json!({
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
            Tool::FetchPage => json!({
                "type": "object",
                "properties": { "url": { "type": "string", "description": "An https address from this answer's search results" } },
                "required": ["url"],
            }),
        }
    }

    /// The tool as the model sees it: name, purpose, and arguments.
    fn definition(self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": self.description(),
                "parameters": self.parameters(),
            },
        })
    }
}

/// Answers one chat request on `client`.
///
/// # Errors
///
/// `AgentError::BadRequest` when the body isn't a JSON object, and upstream
/// or I/O errors before the answer starts. Errors after an event stream has
/// started are sent to the client as a `{"thor":{"error":…}}` event instead.
pub fn answer(client: &mut dyn Client, body: &[u8], upstreams: &Upstreams) -> Outcome {
    let mut fields = body::fields(body)?;
    let web_search = fields
        .remove(WEB_SEARCH_SWITCH)
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let engine = upstreams.serving(body::model(&fields));
    fit(&mut fields, engine);

    let streamed = fields.get(STREAM).and_then(Value::as_bool).unwrap_or(false);
    let model = engine.endpoint();

    match Mode::of(web_search, streamed, brings_tools(&fields)) {
        Mode::Relay => model
            .post(paths::CHAT_COMPLETIONS, &serde_json::to_vec(&fields)?)?
            .relay(client),
        Mode::Stream => as_events(client, &mut |client| {
            stream_round(client, &fields, model)?;

            Ok(())
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

/// Shapes a request for the server that will answer it. An Edge-LLM server
/// needs its own model name and the shape in [`shape`]; llama-server takes
/// what an OpenAI client sends, and its own fields besides.
fn fit(fields: &mut Fields, engine: &Engine) {
    match engine {
        Engine::Edge { model, .. } => {
            body::name(fields, model);
            shape::fit(fields);
        }
        Engine::Llama { .. } => {}
    }
}

/// Runs `body` inside an event stream: an error becomes an error event, and
/// the stream always ends with `[DONE]`.
fn as_events(client: &mut dyn Client, body: &mut dyn FnMut(&mut dyn Write) -> Outcome) -> Outcome {
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
/// After the tool rounds the search hint is taken back out of the system line
/// and the answer rounds run with no tools at all, with a line asking for the
/// answer. A call the model writes there is still run, because the text is
/// streamed as it arrives and cutting the round short would end the answer on a
/// half sentence. Only when no round writes any text does the note name the
/// sources the ledger found, so the page still shows what the search turned up.
fn search_loop(
    client: &mut dyn Write,
    mut fields: Fields,
    upstreams: &Upstreams,
    model: &Endpoint,
) -> Outcome {
    let mut research = Research::new(&user_text(&fields));

    match tool_rounds(client, &mut fields, upstreams, model, &mut research)? {
        Ending::Replied => return Ok(()),
        Ending::Exhausted => (),
    }

    ask(&mut fields, &research.ledger);

    match answer_rounds(client, &mut fields, upstreams, model, &mut research)? {
        Ending::Replied => Ok(()),
        Ending::Exhausted => note(client, &research.ledger),
    }
}

/// How a set of rounds ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// The model stopped asking for tools, so what it wrote is the answer.
    Replied,
    /// The rounds ran out.
    Exhausted,
}

/// The tool rounds: the model may search and read until it answers on its own,
/// the rounds run out, or the search budget is spent.
fn tool_rounds(
    client: &mut dyn Write,
    fields: &mut Fields,
    upstreams: &Upstreams,
    model: &Endpoint,
    research: &mut Research,
) -> Outcome<Ending> {
    for round in 1..=MAX_ROUNDS {
        prepare(fields, Phase::of_tool_round(round));

        let reply = stream_round(client, fields, model)?;

        if reply.cut || reply.calls.is_empty() {
            return Ok(Ending::Replied);
        }

        let results = research.run(client, &reply.calls, upstreams)?;
        append_round(fields, &reply, &results)?;

        if research.out_of_searches() {
            return Ok(Ending::Exhausted);
        }
    }

    Ok(Ending::Exhausted)
}

/// Sets up the answer rounds: the tools go away, the search hint comes back out
/// of the system line, and the numbered sources arrive with the ask as the last
/// user turn — the turn a model that kept calling tools reads.
fn ask(fields: &mut Fields, ledger: &Ledger) {
    prepare(fields, Phase::Answering);
    retract_system(fields, SEARCH_HINT);
    append_system(fields, ANSWER_NUDGE);

    if !ledger.is_empty() {
        append_system(fields, &numbered_sources(ledger));
    }

    append_user(fields, ANSWER_ASK);
}

/// The answer rounds: no tools are offered, so the model writes. A call written
/// anyway is run like any other, because the text is streamed as it arrives and
/// stopping at the first sentence of a summary would end the answer with no
/// links in it.
fn answer_rounds(
    client: &mut dyn Write,
    fields: &mut Fields,
    upstreams: &Upstreams,
    model: &Endpoint,
    research: &mut Research,
) -> Outcome<Ending> {
    for _ in 0..ANSWER_ROUNDS {
        let reply = stream_round(client, fields, model)?;

        if reply.cut || (reply.calls.is_empty() && !reply.content.trim().is_empty()) {
            return Ok(Ending::Replied);
        }

        if reply.calls.is_empty() {
            continue;
        }

        let results = research.run(client, &reply.calls, upstreams)?;
        append_round(fields, &reply, &results)?;
    }

    Ok(Ending::Exhausted)
}

/// The numbered source list the answer cites from.
fn numbered_sources(ledger: &Ledger) -> String {
    format!(
        "Sources found, with the numbers to cite:\n{}",
        ledger.list(MAX_CITED)
    )
}

/// Sends the last-resort line: the rounds are over and no round wrote an answer.
/// When the search found anything, it names what the answer could have used, so
/// the page ends with sources rather than with an excuse.
fn note(client: &mut dyn Write, ledger: &Ledger) -> Outcome {
    let head = "The tool rounds are over and no round wrote an answer.";

    let text = if ledger.is_empty() {
        format!("{head} Ask again, or narrow the question.")
    } else {
        format!(
            "{head} These are the sources the search turned up:\n{}",
            ledger.list(MAX_LISTED)
        )
    };

    response::send_event(
        client,
        &json!({ "choices": [{ "delta": { "content": text } }] }).to_string(),
    )
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
    if let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) {
        messages.push(json!({ "role": "user", "content": line }));
    }
}

/// Takes `line` back out of the system message that [`append_system`] put it in.
///
/// The search hint tells the model to plan sub-questions and call `web_search`,
/// and it stays in the system turn for the tool rounds. Left there for the
/// answer rounds it outranks the ask: a model that is told to search in its
/// system line writes a tool call even when the request offers no tools, which
/// is the loop that ends with no answer at all.
fn retract_system(fields: &mut Fields, line: &str) {
    if let Some(system) = system_line(fields).map(str::to_string) {
        let kept = system
            .split("\n\n")
            .filter(|part| part.trim() != line)
            .collect::<Vec<_>>()
            .join("\n\n");

        set_system(fields, kept);
    }
}

/// Replaces the conversation's system text. A request with no message list is
/// left as it is.
fn set_system(fields: &mut Fields, content: String) {
    let first = fields
        .get_mut(MESSAGES)
        .and_then(Value::as_array_mut)
        .and_then(|messages| messages.first_mut());

    if let Some(first) = first {
        first["content"] = Value::String(content);
    }
}

/// The conversation's system line, when its first message is one holding text.
/// The request keeps at most one, so one lookup is all the callers need.
fn system_line(fields: &Fields) -> Option<&str> {
    fields
        .get(MESSAGES)?
        .as_array()?
        .first()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("system"))?
        .get("content")?
        .as_str()
}

/// What one request offers the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The tools are offered, and this round has to call one.
    Opening,
    /// The tools are offered; the model chooses for itself.
    Searching,
    /// No tools at all: the model writes the answer.
    Answering,
}

impl Phase {
    /// The phase for tool round `round`, counting from one. The first round is
    /// the one that requires a call, so that both models look something up
    /// before they answer.
    fn of_tool_round(round: usize) -> Phase {
        match round {
            1 => Phase::Opening,
            _ => Phase::Searching,
        }
    }
}

/// Writes the phase's tool fields onto the request: the tool list, and the
/// `tool_choice` that makes the first call mandatory.
fn prepare(fields: &mut Fields, phase: Phase) {
    match phase {
        Phase::Opening | Phase::Searching => {
            fields.insert(
                TOOLS.to_string(),
                Tool::ALL.map(Tool::definition).into_iter().collect(),
            );
        }
        Phase::Answering => {
            fields.remove(TOOLS);
        }
    }

    match phase {
        Phase::Opening => {
            fields.insert(TOOL_CHOICE.to_string(), Value::String(REQUIRED.to_string()));
        }
        Phase::Searching | Phase::Answering => {
            fields.remove(TOOL_CHOICE);
        }
    }
}

/// Adds the model's tool calls and their results to the conversation.
fn append_round(fields: &mut Fields, round: &Round, results: &[String]) -> Outcome {
    let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) else {
        return Err(AgentError::bad_request("messages must be a list"));
    };

    messages.push(serde_json::to_value(round.as_message())?);

    for (call, result) in round.calls.iter().zip(results) {
        let record = Added::Tool {
            tool_call_id: &call.id,
            content: result,
        };
        messages.push(serde_json::to_value(record)?);
    }

    Ok(())
}

/// The state the rounds share: where a page may come from, the calls already
/// run, and the sources found with the budgets that bound them.
struct Research {
    /// The hosts this answer may read a page from: those a search returned, and
    /// those the user's own message wrote.
    allowed: Allowed,
    /// The calls already run, keyed by [`signature`].
    ran: Vec<String>,
    /// Every source found, numbered from 1, with the search and page budgets.
    ledger: Ledger,
}

/// The nudge for a call that already ran in this answer.
const ALREADY_RAN: &str = "That tool call already ran in this answer. Use its result.";

impl Research {
    /// A research that may read from the addresses in `text`, the user's own
    /// message; a search adds the hosts its results came from.
    fn new(text: &str) -> Research {
        Research {
            allowed: Allowed::from_text(text),
            ran: Vec::new(),
            ledger: Ledger::new(),
        }
    }

    /// Runs a round's calls in order. Every call gets text back for the model,
    /// whether it ran, repeated one already run, or named no tool at all.
    fn run(
        &mut self,
        client: &mut dyn Write,
        calls: &[ToolCall],
        upstreams: &Upstreams,
    ) -> Outcome<Vec<String>> {
        calls
            .iter()
            .map(|call| self.run_one(client, call, upstreams))
            .collect()
    }

    /// Runs one call. A call that repeats one already run is not run again: the
    /// model is told so and asked to use what it has, which is what stops a
    /// model looping on the same search.
    fn run_one(
        &mut self,
        client: &mut dyn Write,
        call: &ToolCall,
        upstreams: &Upstreams,
    ) -> Outcome<String> {
        let signature = signature(call);

        if self.ran.contains(&signature) {
            return Ok(ALREADY_RAN.to_string());
        }

        self.ran.push(signature);

        match Tool::named(&call.name) {
            Some(Tool::WebSearch) => self.search(client, call, &upstreams.search),
            Some(Tool::FetchPage) => self.read(client, call, &*upstreams.web),
            None => Ok(format!("Unknown tool {}.", call.name)),
        }
    }

    /// Whether the search budget is spent.
    fn out_of_searches(&self) -> bool {
        self.ledger.searches_left(MAX_SEARCHES) == 0
    }

    /// A research round: one search per sub-question the model asked for, widened
    /// by `kind` when it asked for one. Its sources also become the addresses
    /// this answer may read a page from, and every source keeps one number to
    /// cite.
    fn search(
        &mut self,
        client: &mut dyn Write,
        call: &ToolCall,
        searxng: &Endpoint,
    ) -> Outcome<String> {
        let Some(plan) = call.search_plan() else {
            return Ok("The search needs a non-empty query.".to_string());
        };

        let mut blocks: Vec<String> = Vec::new();

        for query in &plan.queries {
            if self.ledger.searches_left(MAX_SEARCHES) == 0 {
                blocks.push(format!(
                    "The search budget is spent ({MAX_SEARCHES} searches). Answer with what you have."
                ));
                break;
            }

            blocks.push(self.search_one(client, query, plan.range, searxng)?);
        }

        blocks.push(format!(
            "Searches used {} of {MAX_SEARCHES}; pages read {} of {MAX_READS}; sources numbered 1 to {}.",
            self.ledger.searches(),
            self.ledger.reads(),
            self.ledger.len()
        ));

        Ok(blocks.join("\n\n"))
    }

    /// Runs one search, tells the page what it found, and returns the results
    /// as the model reads them, or that the search failed.
    fn search_one(
        &mut self,
        client: &mut dyn Write,
        query: &str,
        range: Option<TimeRange>,
        searxng: &Endpoint,
    ) -> Outcome<String> {
        self.ledger.count_search();

        let hits = match search::search(searxng, query, range) {
            Ok(hits) => hits,
            Err(error) => return Ok(format!("The search \"{query}\" failed: {error}")),
        };

        for result in &hits.results {
            self.allowed.add_url(&result.url);
        }

        let event = ThorEvent::Search {
            query,
            results: sources(&hits.results),
        };
        send_thor(client, &event)?;

        Ok(search_block(query, &hits, &mut self.ledger))
    }

    /// Reads one page and its same-site links, up to what is left of the page
    /// budget. Rule 1 of the fetch design is enforced inside
    /// [`fetch::read_recursive`], and every page read is sent to the page so the
    /// answer can list its sources.
    fn read(&mut self, client: &mut dyn Write, call: &ToolCall, web: &dyn Web) -> Outcome<String> {
        let Some(raw) = call.url_argument() else {
            return Ok("The fetch needs an address.".to_string());
        };

        let url = match Url::parse(&raw) {
            Ok(url) => url,
            Err(error) => return Ok(error.to_string()),
        };

        let left = self.ledger.reads_left(MAX_READS);

        if left == 0 {
            return Ok(format!(
                "The page budget is spent ({MAX_READS} pages). Answer with what you have."
            ));
        }

        let report =
            match fetch::read_recursive(web, &url, &self.allowed, left.min(fetch::MAX_PAGES)) {
                Ok(report) => report,
                Err(error) => return Ok(format!("Could not read the page: {error}")),
            };

        self.ledger.count_reads(report.pages.len());

        for page in &report.pages {
            self.ledger.add_page(&page.url, &page.title);

            let event = ThorEvent::Read {
                url: &page.url,
                title: &page.title,
                domain: search::domain_of(&page.url),
            };
            send_thor(client, &event)?;
        }

        Ok(report.text)
    }
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

/// The key a call is remembered by: its name and its arguments.
fn signature(call: &ToolCall) -> String {
    format!("{}\u{1}{}", call.name, call.arguments)
}

fn sources(results: &[SearchResult]) -> Vec<Source<'_>> {
    results
        .iter()
        .map(|result| Source {
            title: &result.title,
            url: &result.url,
            domain: search::domain_of(&result.url),
            published: result.published.as_deref(),
        })
        .collect()
}

/// Streams one model reply to the client, collecting any tool calls. A call the
/// model writes into the text is taken out by [`Sieve`] and runs like a
/// structured one, and the reader never sees the tags.
fn stream_round(client: &mut dyn Write, fields: &Fields, model: &Endpoint) -> Outcome<Round> {
    let request = serde_json::to_vec(fields)?;
    let response = accepted(model.post(paths::CHAT_COMPLETIONS, &request)?)?;
    let mut round = Round::default();
    let mut sieve = Sieve::new();
    let mut streamed = 0_usize;

    for line in response.body.lines() {
        let line = line?;

        let Some(data) = line.strip_prefix(EVENT_PREFIX).map(str::trim) else {
            continue;
        };

        if data == DONE {
            break;
        }

        streamed += relay_chunk(client, data, &mut sieve, &mut round)?;

        if streamed > MAX_ROUND_CHARS {
            round.cut = true;

            let event =
                json!({ "choices": [{ "delta": {}, "finish_reason": "length" }] }).to_string();
            response::send_event(client, &event)?;
            break;
        }
    }

    finish_round(client, sieve, &mut round)?;

    Ok(round)
}

/// The model server's response when it took the request, or what it said
/// when it did not.
fn accepted(response: UpstreamResponse) -> Outcome<UpstreamResponse> {
    if response.is_ok() {
        return Ok(response);
    }

    let status = response.status;

    Err(AgentError::Upstream(format!(
        "model server returned {status}: {}",
        response.text()?
    )))
}

/// Sends one chunk of the model's reply on to the client and adds it to
/// `round`. Returns how many characters of text the chunk carried.
fn relay_chunk(
    client: &mut dyn Write,
    data: &str,
    sieve: &mut Sieve,
    round: &mut Round,
) -> Outcome<usize> {
    let mut value: Value = serde_json::from_str(data)?;
    let length = text_length(&value);

    if let Some(event) = forward(&mut value, data, sieve) {
        response::send_event(client, &event)?;
    }

    round.absorb(serde_json::from_value(value)?);

    Ok(length)
}

/// Ends a round: sends the text the sieve was still holding, and adds the
/// tool calls it took out of the text.
fn finish_round(client: &mut dyn Write, mut sieve: Sieve, round: &mut Round) -> Outcome {
    let tail = sieve.finish();

    if !tail.is_empty() {
        round.content.push_str(&tail);

        let event = json!({ "choices": [{ "delta": { "content": tail } }] }).to_string();
        response::send_event(client, &event)?;
    }

    for (number, call) in sieve.take_calls().into_iter().enumerate() {
        round.calls.push(ToolCall {
            id: format!("text-{number}"),
            name: call.name,
            arguments: call.arguments,
        });
    }

    Ok(())
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

    value["choices"][0]["delta"]["content"] = Value::String(visible.clone());

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

/// The text one chunk adds: the answer and the thinking in it. The round's
/// output budget is spent on both, because either can loop.
fn text_length(value: &Value) -> usize {
    let of = |pointer: &str| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map_or(0, str::len)
    };

    of("/choices/0/delta/content") + of("/choices/0/delta/reasoning_content")
}

#[cfg(test)]
mod tests;
