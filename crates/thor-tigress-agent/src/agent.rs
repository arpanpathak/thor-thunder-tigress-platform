//! The chat request: passed straight to the model server, or, with web search
//! on, run as a loop in which the model may call `web_search` before it
//! answers.
//!
//! ```text
//!   browser ── request ──► agent ── stream ──► llama-server
//!      ▲                     │  tool call: web_search("…")
//!      │                     ▼
//!      │                 SearXNG ── results ──► back to the model, next round
//!      └──── every token, plus a {"thor":{"search":…}} event per search
//! ```
//!
//! Each round is streamed to the browser as it is generated, so answering
//! after a search feels the same as answering without one.

use std::{io::BufRead, net::TcpStream};

use serde_json::{Value, json};

use crate::{error::AgentError, http, search};

/// The most search rounds in one answer; the last round gets no tools, so
/// the model has to answer.
const MAX_ROUNDS: usize = 4;

/// Where to reach the model server and the search engine.
pub struct Upstreams {
    /// The model server, `host:port`.
    pub model: String,
    /// The SearXNG instance, `host:port`.
    pub search: String,
    /// The `Authorization` value the model server expects, if it has a key.
    pub authorization: Option<String>,
}

/// One tool call collected from a streamed reply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// What one streamed round produced besides the tokens already forwarded.
#[derive(Debug, Default)]
struct Round {
    content: String,
    calls: Vec<ToolCall>,
}

/// Answers one chat request. A request that does not ask for streaming, and
/// has web search off, is passed to the model server and answered as plain
/// JSON; otherwise an event stream is written to `browser`.
pub fn chat(browser: &mut TcpStream, body: &[u8], upstreams: &Upstreams) -> Result<(), AgentError> {
    let mut request: Value = serde_json::from_slice(body)?;
    let web = request
        .as_object_mut()
        .and_then(|fields| fields.remove("thor_web_search"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let streamed = request.get("stream").and_then(Value::as_bool).unwrap_or(false);
    if !web && !streamed {
        let response = http::call(
            &upstreams.model,
            "POST",
            "/v1/chat/completions",
            upstreams.authorization.as_deref(),
            Some(&serde_json::to_vec(&request)?),
        )?;
        return http::relay(browser, response);
    }
    request["stream"] = Value::Bool(true);
    http::start_events(browser)?;
    let outcome = match web {
        true => search_loop(browser, request, upstreams),
        false => stream_round(browser, &request, upstreams).map(|_| ()),
    };
    if let Err(error) = &outcome {
        http::send_event(browser, &json!({ "thor": { "error": error.to_string() } }).to_string())?;
    }
    http::send_event(browser, "[DONE]")
}

fn search_loop(browser: &mut TcpStream, mut request: Value, upstreams: &Upstreams) -> Result<(), AgentError> {
    for round in 0..MAX_ROUNDS {
        match round + 1 < MAX_ROUNDS {
            true => request["tools"] = json!([web_search_tool()]),
            false => {
                if let Some(fields) = request.as_object_mut() {
                    fields.remove("tools");
                }
            }
        }
        let produced = stream_round(browser, &request, upstreams)?;
        if produced.calls.is_empty() {
            return Ok(());
        }
        let messages = request["messages"]
            .as_array_mut()
            .ok_or_else(|| AgentError::BadRequest("messages must be a list".to_string()))?;
        messages.push(json!({
            "role": "assistant",
            "content": produced.content,
            "tool_calls": produced.calls.iter().map(|call| json!({
                "id": call.id,
                "type": "function",
                "function": { "name": call.name, "arguments": call.arguments },
            })).collect::<Vec<Value>>(),
        }));
        for call in &produced.calls {
            let result = run_tool(browser, call, upstreams)?;
            messages.push(json!({ "role": "tool", "tool_call_id": call.id, "content": result }));
        }
    }
    Ok(())
}

fn run_tool(browser: &mut TcpStream, call: &ToolCall, upstreams: &Upstreams) -> Result<String, AgentError> {
    if call.name != "web_search" {
        return Ok(format!("Unknown tool {}.", call.name));
    }
    let arguments: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
    let query = arguments.get("query").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if query.is_empty() {
        return Ok("The search needs a non-empty query.".to_string());
    }
    let results = search::search(&upstreams.search, &query).unwrap_or_default();
    let sources: Vec<Value> = results
        .iter()
        .map(|result| json!({ "title": result.title, "url": result.url }))
        .collect();
    http::send_event(browser, &json!({ "thor": { "search": { "query": query, "results": sources } } }).to_string())?;
    Ok(search::as_tool_text(&results))
}

/// Streams one model reply to the browser, collecting any tool calls.
fn stream_round(browser: &mut TcpStream, request: &Value, upstreams: &Upstreams) -> Result<Round, AgentError> {
    let body = serde_json::to_vec(request)?;
    let response = http::call(
        &upstreams.model,
        "POST",
        "/v1/chat/completions",
        upstreams.authorization.as_deref(),
        Some(&body),
    )?;
    let status = response.status;
    if status != 200 {
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
        let chunk: Value = serde_json::from_str(data)?;
        absorb(&chunk, &mut round);
        http::send_event(browser, data)?;
    }
    Ok(round)
}

/// Adds the content and tool-call pieces of one streamed chunk to `round`.
fn absorb(chunk: &Value, round: &mut Round) {
    let delta = &chunk["choices"][0]["delta"];
    if let Some(text) = delta.get("content").and_then(Value::as_str) {
        round.content.push_str(text);
    }
    let pieces = delta.get("tool_calls").and_then(Value::as_array).cloned().unwrap_or_default();
    for piece in pieces {
        let index = piece.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        if round.calls.len() <= index {
            round.calls.resize(index + 1, ToolCall::default());
        }
        let Some(call) = round.calls.get_mut(index) else {
            continue;
        };
        let text = |pointer: &str| piece.pointer(pointer).and_then(Value::as_str).unwrap_or_default();
        call.id.push_str(text("/id"));
        call.name.push_str(text("/function/name"));
        call.arguments.push_str(text("/function/arguments"));
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

    #[test]
    fn assembles_a_tool_call_streamed_in_pieces() {
        let mut round = Round::default();
        let pieces = [
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a1","function":{"name":"web_search","arguments":"{\"que"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ry\":\"rust\"}"}}]}}]}),
            json!({"choices":[{"delta":{"content":"ok"}}]}),
        ];
        pieces.iter().for_each(|chunk| absorb(chunk, &mut round));
        assert_eq!(
            round.calls,
            [ToolCall {
                id: "a1".to_string(),
                name: "web_search".to_string(),
                arguments: "{\"query\":\"rust\"}".to_string(),
            }]
        );
        assert_eq!(round.content, "ok");
    }
}
