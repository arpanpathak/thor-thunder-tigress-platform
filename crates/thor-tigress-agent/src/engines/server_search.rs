//! Anthropic's server-side web search tool (`web_search_20250305` and later),
//! answered here. Claude Code's WebSearch sends a request whose only tool is
//! that one; Anthropic runs the search on its own servers, and no model server
//! here knows the tool, so vLLM answers 400. The agent runs the query on its
//! SearXNG instead and answers in the shape Anthropic's API does.

use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::{
    engines::body::Fields,
    error::Outcome,
    server::response::{self, Client, JSON},
    web::search::{self, SearchResult},
};

/// The prefix of every server-side web search tool's `type`.
const TOOL_PREFIX: &str = "web_search_";

/// The sentence Claude Code puts before the query in the user message.
const ASK_PREFIX: &str = "Perform a web search for the query:";

/// The id the one search is given in the answer.
const TOOL_ID: &str = "srvtoolu_thor_search";

/// Whether `fields` carries a server-side web search tool.
#[must_use]
pub fn requested(fields: &Fields) -> bool {
    fields
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| tools.iter().any(is_search_tool))
}

/// Whether the search tool is the request's only tool, as in Claude Code's
/// WebSearch call: then the request is the search itself. With other tools
/// beside it, the model is asked, without the search tool.
#[must_use]
pub fn only_search(fields: &Fields) -> bool {
    fields
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| !tools.is_empty() && tools.iter().all(is_search_tool))
}

/// Removes every server-side web search tool, for a request that is still
/// passed on, such as a token count.
pub fn strip(fields: &mut Fields) {
    if let Some(tools) = fields.get_mut("tools").and_then(Value::as_array_mut) {
        tools.retain(|tool| !is_search_tool(tool));
    }
}

/// Runs the request's query on `searxng` and answers as Anthropic's API does:
/// a `server_tool_use` block, a `web_search_tool_result` block with the hits,
/// and a text block with their snippets, streamed when the request streams.
///
/// # Errors
///
/// `AgentError::BadRequest` when the request has no query; search and I/O
/// errors otherwise.
pub fn answer(
    client: &mut dyn Client,
    fields: &Fields,
    searxng: &crate::engines::upstream::Endpoint,
) -> Outcome {
    let query = query(fields).ok_or_else(|| {
        crate::error::AgentError::BadRequest("web search request has no query".to_string())
    })?;
    let hits = search::search_all(searxng, std::slice::from_ref(&query), None)
        .into_iter()
        .next()
        .unwrap_or_else(|| Ok(search::Hits::default()))?;

    let model = fields.get("model").cloned().unwrap_or(Value::Null);
    let blocks = blocks(&query, &hits.results);

    if fields.get("stream").and_then(Value::as_bool) == Some(true) {
        stream(client, &model, &blocks)
    } else {
        let message = json!({
            "id": "msg_thor_search",
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": blocks,
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": usage(),
        });

        response::respond(client, StatusCode::OK, JSON, message.to_string().as_bytes())
    }
}

fn is_search_tool(tool: &Value) -> bool {
    tool.get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.starts_with(TOOL_PREFIX))
}

/// The last user message's text, without Claude Code's lead-in sentence.
fn query(fields: &Fields) -> Option<String> {
    let message = fields
        .get("messages")?
        .as_array()?
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))?;

    let text = match message.get("content")? {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };

    let query = text.trim().strip_prefix(ASK_PREFIX).unwrap_or(&text).trim();

    (!query.is_empty()).then(|| query.to_string())
}

fn blocks(query: &str, results: &[SearchResult]) -> Vec<Value> {
    let found: Vec<Value> = results.iter().map(found).collect();

    vec![
        json!({ "type": "server_tool_use", "id": TOOL_ID, "name": "web_search", "input": { "query": query } }),
        json!({ "type": "web_search_tool_result", "tool_use_id": TOOL_ID, "content": found }),
        json!({ "type": "text", "text": summary(query, results) }),
    ]
}

fn found(result: &SearchResult) -> Value {
    json!({
        "type": "web_search_result",
        "title": result.title,
        "url": result.url,
        "encrypted_content": "",
        "page_age": result.published,
    })
}

/// The results as numbered lines the model reads: title, address, snippet.
fn summary(query: &str, results: &[SearchResult]) -> String {
    if results.is_empty() {
        return format!("The search for \"{query}\" returned no results.");
    }

    results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            format!(
                "{}. {} ({})\n{}",
                index + 1,
                result.title,
                result.url,
                result.snippet
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn usage() -> Value {
    json!({ "input_tokens": 0, "output_tokens": 0, "server_tool_use": { "web_search_requests": 1 } })
}

/// The blocks as Anthropic's event stream: each block starts, carries its
/// content, and stops, between `message_start` and `message_stop`.
fn stream(client: &mut dyn Client, model: &Value, blocks: &[Value]) -> Outcome {
    response::start_events(client)?;

    let start = json!({
        "type": "message_start",
        "message": {
            "id": "msg_thor_search", "type": "message", "role": "assistant", "model": model,
            "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": { "input_tokens": 0, "output_tokens": 0 },
        },
    });
    event(client, &start)?;

    blocks
        .iter()
        .enumerate()
        .try_for_each(|(index, block)| stream_block(client, index, block))?;

    let end = json!({
        "type": "message_delta",
        "delta": { "stop_reason": "end_turn", "stop_sequence": null },
        "usage": usage(),
    });
    event(client, &end)?;
    event(client, &json!({ "type": "message_stop" }))
}

/// One block as `content_block_start`, an optional delta, and `content_block_stop`.
fn stream_block(client: &mut dyn Client, index: usize, block: &Value) -> Outcome {
    let (opening, delta) = split(block);

    event(
        client,
        &json!({ "type": "content_block_start", "index": index, "content_block": opening }),
    )?;
    if let Some(delta) = delta {
        event(
            client,
            &json!({ "type": "content_block_delta", "index": index, "delta": delta }),
        )?;
    }
    event(
        client,
        &json!({ "type": "content_block_stop", "index": index }),
    )
}

/// A block as it opens in a stream and the delta that fills it: a tool use
/// opens with empty input and gets its input as JSON, a text block opens empty
/// and gets its text; a search result arrives whole.
fn split(block: &Value) -> (Value, Option<Value>) {
    let field = |name: &str| block.get(name).cloned().unwrap_or(Value::Null);

    match block.get("type").and_then(Value::as_str) {
        Some("server_tool_use") => {
            let mut opening = block.clone();
            opening["input"] = json!({});
            let input = field("input").to_string();
            (
                opening,
                Some(json!({ "type": "input_json_delta", "partial_json": input })),
            )
        }
        Some("text") => (
            json!({ "type": "text", "text": "" }),
            Some(json!({ "type": "text_delta", "text": field("text") })),
        ),
        _ => (block.clone(), None),
    }
}

/// One Anthropic event: its type named on an `event:` line, then its data.
fn event(client: &mut dyn Client, data: &Value) -> Outcome {
    let kind = data
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");

    client.write_all(format!("event: {kind}\ndata: {data}\n\n").as_bytes())?;

    Ok(client.flush()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(value: Value) -> Fields {
        match value {
            Value::Object(map) => map,
            _ => Fields::new(),
        }
    }

    #[test]
    fn finds_the_search_tool_and_the_query() {
        let request = fields(json!({
            "tools": [{ "type": "web_search_20250305", "name": "web_search", "max_uses": 8 }],
            "messages": [{ "role": "user", "content": "Perform a web search for the query: cuda-oxide docs" }],
        }));

        assert!(requested(&request));
        assert_eq!(query(&request).as_deref(), Some("cuda-oxide docs"));
    }

    #[test]
    fn search_beside_other_tools_is_not_the_search_itself() {
        let alone =
            fields(json!({ "tools": [{ "type": "web_search_20250305", "name": "web_search" }] }));
        let mixed = fields(json!({
            "tools": [{ "type": "web_search_20250305", "name": "web_search" }, { "name": "Bash", "input_schema": {} }],
        }));

        assert!(only_search(&alone));
        assert!(!only_search(&mixed));
        assert!(requested(&mixed));
    }

    #[test]
    fn ordinary_tools_are_not_the_search_tool() {
        let request = fields(json!({ "tools": [{ "name": "Bash", "input_schema": {} }] }));

        assert!(!requested(&request));
    }

    #[test]
    fn strip_keeps_the_other_tools() {
        let mut request = fields(json!({
            "tools": [{ "type": "web_search_20250305", "name": "web_search" }, { "name": "Bash", "input_schema": {} }],
        }));

        strip(&mut request);

        assert_eq!(
            request["tools"],
            json!([{ "name": "Bash", "input_schema": {} }])
        );
    }

    #[test]
    fn blocks_carry_the_results_and_their_snippets() {
        let results = vec![SearchResult {
            title: "cuda-oxide".to_string(),
            url: "https://nvidia.github.io/cuda-rust/cuda-oxide/".to_string(),
            snippet: "A Rust to PTX compiler".to_string(),
            published: None,
        }];

        let blocks = blocks("cuda-oxide", &results);

        assert_eq!(
            blocks[1]["content"][0]["url"],
            json!("https://nvidia.github.io/cuda-rust/cuda-oxide/")
        );
        assert!(
            blocks[2]["text"]
                .as_str()
                .is_some_and(|text| text.contains("A Rust to PTX compiler"))
        );
    }
}
