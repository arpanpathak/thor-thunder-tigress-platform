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
//! something up before they answer; later rounds go back to `auto`.

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
    response::{self, DONE, EVENT_PREFIX},
    search::{self, SearchResult, TimeRange},
    upstream::Endpoint,
};

/// Rounds per answer; the last one gets no tools, so the model has to answer.
const MAX_ROUNDS: usize = 4;

/// Tool calls kept per round; more are ignored.
const MAX_CALLS: usize = 8;

/// The page's switch for web search; removed before the model sees the request.
const WEB_SEARCH_SWITCH: &str = "thor_web_search";

/// The line added under the switch. It names the tools so a model that would
/// answer from memory still reaches for one, and says what `time_range` is for.
const SEARCH_HINT: &str = concat!(
    "Web search is available. Use the web_search tool for current events, jobs, ",
    "prices, releases, or facts you are not certain of; set its time_range to day, ",
    "week, month or year when the answer depends on what is recent. Use the ",
    "fetch_page_content_recursive tool to read a page from the results when the ",
    "snippet is not enough."
);

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
                "Search the web. Returns titles, addresses, dates and short snippets of the top results. Set time_range to day, week, month or year when the answer depends on what is recent, such as jobs or other new postings.",
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "What to search for" },
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

/// The arguments of one `web_search` call.
#[derive(Deserialize)]
struct SearchArguments {
    query: String,
    #[serde(default)]
    time_range: Option<String>,
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
/// without a tool call or the rounds run out.
///
/// The first round requires a call, so both models look something up before
/// they answer; later rounds leave the choice to the model.
fn search_loop(
    client: &mut dyn Write,
    mut fields: Fields,
    upstreams: &Upstreams,
    model: &Endpoint,
) -> Outcome {
    let mut allowed = Allowed::from_text(&user_text(&fields));

    for round_number in 1..=MAX_ROUNDS {
        offer_tools(&mut fields, round_number < MAX_ROUNDS);
        require_tool(&mut fields, round_number == 1);
        let round = stream_round(client, &fields, model)?;

        if round.calls.is_empty() {
            return Ok(());
        }

        let results = round
            .calls
            .iter()
            .map(|call| run_tool(client, call, upstreams, &mut allowed))
            .collect::<Outcome<Vec<String>>>()?;
        append_round(&mut fields, &round, &results)?;
    }
    Ok(())
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
    let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) else {
        return;
    };

    let first_is_system = messages
        .first()
        .is_some_and(|message| message.get("role").and_then(Value::as_str) == Some("system"));

    if !first_is_system {
        messages.insert(0, json!({ "role": "system", "content": SEARCH_HINT }));

        return;
    }

    let merged = messages[0]
        .get("content")
        .and_then(Value::as_str)
        .map(|content| format!("{content}\n\n{SEARCH_HINT}"));

    if let Some(content) = merged {
        messages[0]["content"] = Value::String(content);
    }
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
) -> Outcome<String> {
    match Tool::named(&call.name) {
        Some(Tool::WebSearch) => web_search(client, call, &upstreams.search, allowed),
        Some(Tool::FetchPage) => fetch_page(client, call, &*upstreams.web, allowed),
        None => Ok(format!("Unknown tool {}.", call.name)),
    }
}

/// A search, with the recency it asked for; its sources also become the
/// addresses this answer may read a page from.
fn web_search(
    client: &mut dyn Write,
    call: &ToolCall,
    searxng: &Endpoint,
    allowed: &mut Allowed,
) -> Outcome<String> {
    let Some((query, range)) = call.search_arguments() else {
        return Ok("The search needs a non-empty query.".to_string());
    };

    let results = search::search(searxng, &query, range).unwrap_or_default();
    for result in &results {
        allowed.add_url(&result.url);
    }
    let event = ThorEvent::Search {
        query: &query,
        results: sources(&results),
    };
    send_thor(client, &event)?;
    Ok(search::as_tool_text(&results))
}

/// Reads one page and its same-site links. Rule 1 of the fetch design is
/// enforced inside [`fetch::read_recursive`], and every page read is sent to
/// the page so the answer can list its sources.
fn fetch_page(
    client: &mut dyn Write,
    call: &ToolCall,
    web: &dyn Web,
    allowed: &Allowed,
) -> Outcome<String> {
    let Some(raw) = call.url_argument() else {
        return Ok("The fetch needs an address.".to_string());
    };
    let url = match Url::parse(&raw) {
        Ok(url) => url,
        Err(error) => return Ok(error.to_string()),
    };
    let report = match fetch::read_recursive(web, &url, allowed) {
        Ok(report) => report,
        Err(error) => return Ok(format!("Could not read the page: {error}")),
    };
    for page in &report.pages {
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

/// Streams one model reply to the client, collecting any tool calls.
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

    for line in response.body.lines() {
        let line = line?;
        let Some(data) = line.strip_prefix(EVENT_PREFIX).map(str::trim) else {
            continue;
        };

        if data == DONE {
            break;
        }

        round.absorb(serde_json::from_str(data)?);
        response::send_event(client, data)?;
    }
    Ok(round)
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

    /// The `query` and `time_range` of a search, trimmed; `None` when the
    /// query is missing or empty. An unknown range is treated as none.
    fn search_arguments(&self) -> Option<(String, Option<TimeRange>)> {
        let arguments: SearchArguments = self.arguments()?;
        let query = arguments.query.trim();
        (!query.is_empty()).then(|| {
            (
                query.to_string(),
                arguments.time_range.as_deref().and_then(TimeRange::of),
            )
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
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":name,"arguments":arguments}}]}}]})
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
            round.calls[0].search_arguments(),
            Some(("rust".to_string(), None))
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
    fn a_tool_call_must_carry_its_argument() {
        let call = |arguments: &str| ToolCall {
            arguments: arguments.to_string(),
            ..ToolCall::default()
        };
        assert_eq!(
            call(r#"{"query":"  rust  "}"#).search_arguments(),
            Some(("rust".to_string(), None))
        );
        assert_eq!(
            call(r#"{"query":"jobs","time_range":"week"}"#).search_arguments(),
            Some(("jobs".to_string(), Some(TimeRange::Week)))
        );
        assert_eq!(
            call(r#"{"query":"jobs","time_range":"forever"}"#).search_arguments(),
            Some(("jobs".to_string(), None))
        );
        assert_eq!(call(r#"{"query":"  "}"#).search_arguments(), None);
        assert_eq!(call(r#"{"q":"rust"}"#).search_arguments(), None);
        assert_eq!(call("not json").search_arguments(), None);
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
    fn the_last_round_has_no_tools() -> Outcome {
        let call = call_event("other", "{}");
        let model = FakeServer::start(vec![event_stream(&[&call]); MAX_ROUNDS])?;
        answer(
            &mut Vec::new(),
            br#"{"messages":[],"thor_web_search":true}"#,
            &upstreams(&model, &idle()?),
        )?;
        let seen = model.requests()?;
        assert_eq!(seen.len(), MAX_ROUNDS);
        assert!(seen[MAX_ROUNDS - 2].contains(r#""tools""#));
        assert!(!seen[MAX_ROUNDS - 1].contains(r#""tools""#));
        assert!(seen[1].contains("Unknown tool other."));
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
