//! `/v1/chat/completions`: relayed as it is, streamed, or, with web search
//! on, run as a loop in which the model may call a tool before it answers.
//!
//! ```text
//!   client ── request ──► server ── stream ──► llama-server
//!      ▲                    │  tool call: web_search("…")
//!      │                    ▼
//!      │                SearXNG ── results ──► back to the model, next round
//!      └──── every token, plus a {"thor":{"search":…}} event per search
//! ```

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    config::Upstreams,
    error::{AgentError, Outcome},
    paths,
    response::{self, DONE, EVENT_PREFIX},
    search::{self, SearchResult},
    upstream::Endpoint,
};

/// Rounds per answer; the last one gets no tools, so the model has to answer.
const MAX_ROUNDS: usize = 4;

/// Tool calls kept per round; more are ignored.
const MAX_CALLS: usize = 8;

/// The page's switch for web search; removed before the model sees the request.
const WEB_SEARCH_SWITCH: &str = "thor_web_search";

/// The line added under the switch, so a model that would answer from memory
/// still reaches for the tool when the answer may have moved.
const SEARCH_HINT: &str = concat!(
    "When web search is available, use the web_search tool for anything about ",
    "current events, releases, prices, or facts you are not certain of."
);

/// The request field asking for a streamed answer.
const STREAM: &str = "stream";

/// The request field naming the model, which picks the engine.
const MODEL: &str = "model";

/// The request field listing the tools the model may call.
const TOOLS: &str = "tools";

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
}

impl Tool {
    /// Every tool, in the order the model is told about them.
    const ALL: [Tool; 1] = [Tool::WebSearch];

    fn name(self) -> &'static str {
        match self {
            Tool::WebSearch => "web_search",
        }
    }

    fn named(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// The tool as the model sees it: name, purpose, and arguments.
    fn definition(self) -> Value {
        let (description, parameters) = match self {
            Tool::WebSearch => (
                "Search the web. Returns titles, addresses and short snippets of the top results.",
                json!({
                    "type": "object",
                    "properties": { "query": { "type": "string", "description": "What to search for" } },
                    "required": ["query"],
                }),
            ),
        };
        json!({ "type": "function", "function": { "name": self.name(), "description": description, "parameters": parameters } })
    }
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
                search_loop(client, fields.clone(), model, &upstreams.search)
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
fn search_loop(
    client: &mut dyn Write,
    mut fields: Fields,
    model: &Endpoint,
    search: &Endpoint,
) -> Outcome {
    for round_number in 1..=MAX_ROUNDS {
        offer_tools(&mut fields, round_number < MAX_ROUNDS);
        let round = stream_round(client, &fields, model)?;

        if round.calls.is_empty() {
            return Ok(());
        }

        let results = round
            .calls
            .iter()
            .map(|call| run_tool(client, call, search))
            .collect::<Outcome<Vec<String>>>()?;
        append_round(&mut fields, &round, &results)?;
    }
    Ok(())
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
fn run_tool(client: &mut dyn Write, call: &ToolCall, searxng: &Endpoint) -> Outcome<String> {
    match Tool::named(&call.name) {
        Some(Tool::WebSearch) => web_search(client, call, searxng),
        None => Ok(format!("Unknown tool {}.", call.name)),
    }
}

fn web_search(client: &mut dyn Write, call: &ToolCall, searxng: &Endpoint) -> Outcome<String> {
    let Some(query) = call.query() else {
        return Ok("The search needs a non-empty query.".to_string());
    };

    let results = search::search(searxng, &query).unwrap_or_default();
    send_thor(
        client,
        &ThorEvent::Search {
            query: &query,
            results: sources(&results),
        },
    )?;
    Ok(search::as_tool_text(&results))
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

    /// The `query` argument, trimmed; `None` when missing or empty.
    fn query(&self) -> Option<String> {
        #[derive(Deserialize)]
        struct Arguments {
            query: String,
        }
        let arguments: Arguments = serde_json::from_str(&self.arguments).ok()?;
        let query = arguments.query.trim();
        (!query.is_empty()).then(|| query.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, event_stream, json_response};

    fn upstreams(model: &FakeServer, search: &FakeServer) -> Upstreams {
        Upstreams {
            model: Endpoint::new(model.address(), None),
            engines: Vec::new(),
            search: Endpoint::new(search.address(), None),
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
        assert_eq!(Tool::named("fetch_page"), None);
        assert_eq!(
            Tool::WebSearch.definition()["function"]["name"],
            "web_search"
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
        assert_eq!(round.calls[0].query().as_deref(), Some("rust"));
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
    fn a_query_must_be_present_and_non_empty() {
        let call = |arguments: &str| ToolCall {
            arguments: arguments.to_string(),
            ..ToolCall::default()
        };
        assert_eq!(
            call(r#"{"query":"  rust  "}"#).query().as_deref(),
            Some("rust")
        );
        assert_eq!(call(r#"{"query":"  "}"#).query(), None);
        assert_eq!(call(r#"{"q":"rust"}"#).query(), None);
        assert_eq!(call("not json").query(), None);
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
