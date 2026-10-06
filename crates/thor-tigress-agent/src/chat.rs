//! `/v1/chat/completions`: relayed as it is, streamed, or, with web search
//! on, run as a loop in which the model may call `web_search` before it
//! answers.
//!
//! ```text
//!   client ── request ──► agent ── stream ──► llama-server
//!      ▲                    │  tool call: web_search("…")
//!      │                    ▼
//!      │                SearXNG ── results ──► back to the model, next round
//!      └──── every token, plus a {"thor":{"search":…}} event per search
//! ```

use std::io::{BufRead, Write};

use serde_json::{Map, Value, json};

use crate::{config::Upstreams, error::AgentError, response, search, upstream::Endpoint};

/// The model server's chat path.
const COMPLETIONS: &str = "/v1/chat/completions";

/// Rounds per answer; the last one gets no tools, so the model has to answer.
const MAX_ROUNDS: usize = 4;

/// Tool calls kept per round; more are ignored.
const MAX_CALLS: usize = 8;

/// A chat request's top-level fields.
type Fields = Map<String, Value>;

/// How a chat request is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Passed to the model server and its answer copied back as it is.
    Relay,
    /// Streamed to the client as events.
    Stream,
    /// Streamed, with the `web_search` tool available.
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

/// One tool call, assembled from the pieces a streamed reply sends.
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

/// Answers one chat request on `client`.
///
/// # Errors
///
/// `AgentError::BadRequest` when the body isn't a JSON object, and upstream
/// or I/O errors before the answer starts. Errors after an event stream has
/// started are sent to the client as a `{"thor":{"error":…}}` event instead.
pub fn answer(client: &mut impl Write, body: &[u8], upstreams: &Upstreams) -> Result<(), AgentError> {
    let Value::Object(mut fields) = serde_json::from_slice(body)? else {
        return Err(AgentError::bad_request("the body must be a JSON object"));
    };
    let web_search = fields.remove("thor_web_search").and_then(|value| value.as_bool()).unwrap_or(false);
    let streamed = fields.get("stream").and_then(Value::as_bool).unwrap_or(false);
    match Mode::of(web_search, streamed) {
        Mode::Relay => upstreams.model.post(COMPLETIONS, &serde_json::to_vec(&fields)?)?.relay(client),
        Mode::Stream => as_events(client, |client| stream_round(client, &fields, &upstreams.model).map(drop)),
        Mode::Search => {
            fields.insert("stream".to_string(), Value::Bool(true));
            as_events(client, |client| search_loop(client, fields, upstreams))
        }
    }
}

/// Runs `body` inside an event stream: an error becomes an error event, and
/// the stream always ends with `[DONE]`.
fn as_events<W: Write>(client: &mut W, body: impl FnOnce(&mut W) -> Result<(), AgentError>) -> Result<(), AgentError> {
    response::start_events(client)?;
    if let Err(error) = body(client) {
        response::send_event(client, &json!({ "thor": { "error": error.to_string() } }).to_string())?;
    }
    response::send_event(client, "[DONE]")
}

/// Asks the model, runs the tools it calls, and asks again, until it answers
/// without a tool call or the rounds run out.
fn search_loop(client: &mut impl Write, mut fields: Fields, upstreams: &Upstreams) -> Result<(), AgentError> {
    for round_number in 1..=MAX_ROUNDS {
        offer_tools(&mut fields, round_number < MAX_ROUNDS);
        let round = stream_round(client, &fields, &upstreams.model)?;
        if round.calls.is_empty() {
            return Ok(());
        }
        let results = round
            .calls
            .iter()
            .map(|call| run_tool(client, call, &upstreams.search))
            .collect::<Result<Vec<String>, AgentError>>()?;
        append_round(&mut fields, &round, &results)?;
    }
    Ok(())
}

fn offer_tools(fields: &mut Fields, offered: bool) {
    if offered {
        fields.insert("tools".to_string(), json!([web_search_tool()]));
    } else {
        fields.remove("tools");
    }
}

/// Adds the model's tool calls and their results to the conversation.
fn append_round(fields: &mut Fields, round: &Round, results: &[String]) -> Result<(), AgentError> {
    let Some(messages) = fields.get_mut("messages").and_then(Value::as_array_mut) else {
        return Err(AgentError::bad_request("messages must be a list"));
    };
    messages.push(round.as_message());
    for (call, result) in round.calls.iter().zip(results) {
        messages.push(json!({ "role": "tool", "tool_call_id": call.id, "content": result }));
    }
    Ok(())
}

/// Runs one tool call and tells the client about the search.
fn run_tool(client: &mut impl Write, call: &ToolCall, searxng: &Endpoint) -> Result<String, AgentError> {
    if call.name != "web_search" {
        return Ok(format!("Unknown tool {}.", call.name));
    }
    let Some(query) = call.query() else {
        return Ok("The search needs a non-empty query.".to_string());
    };
    let results = search::search(searxng, &query).unwrap_or_default();
    let sources: Vec<Value> = results
        .iter()
        .map(|result| json!({ "title": result.title, "url": result.url }))
        .collect();
    response::send_event(client, &json!({ "thor": { "search": { "query": query, "results": sources } } }).to_string())?;
    Ok(search::as_tool_text(&results))
}

/// Streams one model reply to the client, collecting any tool calls.
fn stream_round(client: &mut impl Write, fields: &Fields, model: &Endpoint) -> Result<Round, AgentError> {
    let response = model.post(COMPLETIONS, &serde_json::to_vec(fields)?)?;
    if response.status != 200 {
        let status = response.status;
        return Err(AgentError::Upstream(format!("model server returned {status}: {}", response.text()?)));
    }
    let mut round = Round::default();
    for line in response.body.lines() {
        let line = line?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        round.absorb(&serde_json::from_str(data)?);
        response::send_event(client, data)?;
    }
    Ok(round)
}

impl Round {
    /// Adds the text and tool-call pieces of one streamed chunk.
    fn absorb(&mut self, chunk: &Value) {
        let Some(delta) = chunk.pointer("/choices/0/delta") else {
            return;
        };
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            self.content.push_str(text);
        }
        let pieces = delta.get("tool_calls").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        for piece in pieces {
            let index = piece.get("index").and_then(Value::as_u64).and_then(|index| usize::try_from(index).ok());
            let Some(index) = index.filter(|&index| index < MAX_CALLS) else {
                continue;
            };
            if self.calls.len() <= index {
                self.calls.resize_with(index + 1, ToolCall::default);
            }
            self.calls[index].extend(piece);
        }
    }

    /// The assistant message that asked for this round's tool calls.
    fn as_message(&self) -> Value {
        let calls: Vec<Value> = self
            .calls
            .iter()
            .map(|call| {
                json!({ "id": call.id, "type": "function", "function": { "name": call.name, "arguments": call.arguments } })
            })
            .collect();
        json!({ "role": "assistant", "content": self.content, "tool_calls": calls })
    }
}

impl ToolCall {
    /// Appends one streamed piece: the id, the name and the arguments arrive in parts.
    fn extend(&mut self, piece: &Value) {
        let text = |pointer: &str| piece.pointer(pointer).and_then(Value::as_str).unwrap_or_default();
        self.id.push_str(text("/id"));
        self.name.push_str(text("/function/name"));
        self.arguments.push_str(text("/function/arguments"));
    }

    /// The `query` argument, trimmed; `None` when missing or empty.
    fn query(&self) -> Option<String> {
        let arguments: Value = serde_json::from_str(&self.arguments).ok()?;
        let query = arguments.get("query")?.as_str()?.trim();
        (!query.is_empty()).then(|| query.to_string())
    }
}

/// The `web_search` tool as the model sees it.
fn web_search_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "web_search",
            "description": "Search the web. Returns titles, addresses and short snippets of the top results.",
            "parameters": {
                "type": "object",
                "properties": { "query": { "type": "string", "description": "What to search for" } },
                "required": ["query"],
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, event_stream, json_response};

    fn upstreams(model: &FakeServer, search: &FakeServer) -> Upstreams {
        Upstreams {
            model: Endpoint::new(model.address(), None),
            search: Endpoint::new(search.address(), None),
        }
    }

    fn idle() -> Result<FakeServer, AgentError> {
        FakeServer::start(Vec::new())
    }

    fn events(client: &[u8]) -> Vec<String> {
        String::from_utf8_lossy(client)
            .lines()
            .filter_map(|line| line.strip_prefix("data: ").map(ToString::to_string))
            .collect()
    }

    #[test]
    fn the_mode_is_a_truth_table() {
        assert_eq!(Mode::of(true, true), Mode::Search);
        assert_eq!(Mode::of(true, false), Mode::Search);
        assert_eq!(Mode::of(false, true), Mode::Stream);
        assert_eq!(Mode::of(false, false), Mode::Relay);
    }

    #[test]
    fn assembles_a_tool_call_streamed_in_pieces() {
        let mut round = Round::default();
        round.absorb(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a1","function":{"name":"web_search","arguments":"{\"que"}}]}}]}));
        round.absorb(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ry\":\"rust\"}"}}]}}]}));
        round.absorb(&json!({"choices":[{"delta":{"content":"ok"}}]}));
        round.absorb(&json!({"choices":[]}));
        assert_eq!(round.content, "ok");
        assert_eq!(round.calls.len(), 1);
        assert_eq!(round.calls[0].query().as_deref(), Some("rust"));
    }

    #[test]
    fn ignores_tool_calls_past_the_limit() {
        let mut round = Round::default();
        round.absorb(&json!({"choices":[{"delta":{"tool_calls":[{"index":1_000_000,"id":"x"}]}}]}));
        assert_eq!(round.calls, []);
    }

    #[test]
    fn a_query_must_be_present_and_non_empty() {
        let call = |arguments: &str| ToolCall { arguments: arguments.to_string(), ..ToolCall::default() };
        assert_eq!(call(r#"{"query":"  rust  "}"#).query().as_deref(), Some("rust"));
        assert_eq!(call(r#"{"query":"  "}"#).query(), None);
        assert_eq!(call(r#"{"q":"rust"}"#).query(), None);
        assert_eq!(call("not json").query(), None);
    }

    #[test]
    fn relays_a_request_that_does_not_stream() -> Result<(), AgentError> {
        let (model, search) = (FakeServer::start(vec![json_response(r#"{"choices":[]}"#)])?, idle()?);
        let mut client = Vec::new();
        answer(&mut client, br#"{"messages":[]}"#, &upstreams(&model, &search))?;
        assert!(String::from_utf8_lossy(&client).ends_with(r#"{"choices":[]}"#));
        assert!(model.requests()?[0].ends_with(r#"{"messages":[]}"#));
        Ok(())
    }

    #[test]
    fn streams_tokens_and_ends_with_done() -> Result<(), AgentError> {
        let token = r#"{"choices":[{"delta":{"content":"hi"}}]}"#;
        let (model, search) = (FakeServer::start(vec![event_stream(&[token])])?, idle()?);
        let mut client = Vec::new();
        answer(&mut client, br#"{"messages":[],"stream":true}"#, &upstreams(&model, &search))?;
        assert_eq!(events(&client), [token, "[DONE]"]);
        Ok(())
    }

    #[test]
    fn searches_then_answers() -> Result<(), AgentError> {
        let call = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"web_search","arguments":"{\"query\":\"rust\"}"}}]}}]}"#;
        let reply = r#"{"choices":[{"delta":{"content":"Rust 1.99"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[call]), event_stream(&[reply])])?;
        let search = FakeServer::start(vec![json_response(r#"{"results":[{"title":"Rust","url":"https://r","content":"new"}]}"#)])?;
        let mut client = Vec::new();
        answer(&mut client, br#"{"messages":[{"role":"user","content":"news?"}],"thor_web_search":true}"#, &upstreams(&model, &search))?;
        let seen = model.requests()?;
        assert!(seen[0].contains(r#""tools""#) && seen[0].contains(r#""stream":true"#));
        assert!(seen[1].contains(r#""role":"tool""#) && seen[1].contains("[1] Rust"));
        let sent = events(&client);
        assert_eq!(sent.first().map(String::as_str), Some(call));
        assert!(sent.iter().any(|event| event.contains(r#""search":{"query":"rust""#)));
        assert_eq!(sent.last().map(String::as_str), Some("[DONE]"));
        Ok(())
    }

    #[test]
    fn the_last_round_has_no_tools() -> Result<(), AgentError> {
        let call = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"other","arguments":"{}"}}]}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[call]); MAX_ROUNDS])?;
        let mut client = Vec::new();
        answer(&mut client, br#"{"messages":[],"thor_web_search":true}"#, &upstreams(&model, &idle()?))?;
        let seen = model.requests()?;
        assert_eq!(seen.len(), MAX_ROUNDS);
        assert!(seen[MAX_ROUNDS - 2].contains(r#""tools""#));
        assert!(!seen[MAX_ROUNDS - 1].contains(r#""tools""#));
        assert!(seen[1].contains("Unknown tool other."));
        Ok(())
    }

    #[test]
    fn a_model_error_becomes_an_error_event() -> Result<(), AgentError> {
        let (model, search) = (FakeServer::start(vec!["HTTP/1.1 500 Oops\r\n\r\nbroken".to_string()])?, idle()?);
        let mut client = Vec::new();
        answer(&mut client, br#"{"messages":[],"stream":true}"#, &upstreams(&model, &search))?;
        let sent = events(&client);
        assert_eq!(sent.len(), 2);
        assert!(sent[0].contains("model server returned 500: broken"));
        Ok(())
    }

    #[test]
    fn rejects_a_body_that_is_not_an_object() -> Result<(), AgentError> {
        let (model, search) = (idle()?, idle()?);
        let outcome = answer(&mut Vec::new(), b"[]", &upstreams(&model, &search));
        assert!(outcome.is_err_and(|error| error.to_string() == "bad request: the body must be a JSON object"));
        Ok(())
    }

    #[test]
    fn a_search_without_messages_is_reported() -> Result<(), AgentError> {
        let call = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"web_search","arguments":"{}"}}]}}]}"#;
        let (model, search) = (FakeServer::start(vec![event_stream(&[call])])?, idle()?);
        let mut client = Vec::new();
        answer(&mut client, br#"{"thor_web_search":true}"#, &upstreams(&model, &search))?;
        assert!(events(&client).iter().any(|event| event.contains("messages must be a list")));
        Ok(())
    }
}
