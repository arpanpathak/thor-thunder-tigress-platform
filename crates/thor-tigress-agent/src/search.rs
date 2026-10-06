//! Web search through a SearXNG instance on localhost.

use serde_json::Value;

use crate::{error::AgentError, upstream::Endpoint};

/// How many results are given to the model per search.
const MAX_RESULTS: usize = 6;

/// The longest snippet kept per result, in characters.
const MAX_SNIPPET: usize = 400;

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// The page title.
    pub title: String,
    /// The page address.
    pub url: String,
    /// The text the search engine shows under the title.
    pub snippet: String,
}

/// Searches `query` on the SearXNG instance at `searxng`.
///
/// # Errors
///
/// `AgentError::Upstream` when SearXNG can't be reached or answers with an
/// error; `AgentError::Json` when its answer isn't JSON.
pub fn search(searxng: &Endpoint, query: &str) -> Result<Vec<SearchResult>, AgentError> {
    let response = searxng.get(&format!("/search?q={}&format=json", encode(query)))?;
    if response.status != 200 {
        return Err(AgentError::Upstream(format!("search returned {}", response.status)));
    }
    let answer: Value = serde_json::from_str(&response.text()?)?;
    let results = answer
        .get("results")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    Ok(results.iter().filter_map(parse_result).take(MAX_RESULTS).collect())
}

/// One SearXNG result; `None` when it has no address.
fn parse_result(result: &Value) -> Option<SearchResult> {
    let text = |name: &str| result.get(name).and_then(Value::as_str).unwrap_or_default().trim();
    let url = text("url");
    if url.is_empty() {
        return None;
    }
    Some(SearchResult {
        title: text("title").to_string(),
        url: url.to_string(),
        snippet: text("content").chars().take(MAX_SNIPPET).collect(),
    })
}

/// The results as the text the model reads: one numbered entry per result.
#[must_use]
pub fn as_tool_text(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results.".to_string();
    }
    let entries: Vec<String> = results
        .iter()
        .zip(1..)
        .map(|(result, number)| format!("[{number}] {}\n{}\n{}", result.title, result.url, result.snippet))
        .collect();
    entries.join("\n\n")
}

/// Percent-encodes `text` for a query string.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => char::from(byte).to_string(),
            b' ' => "+".to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, json_response};

    #[test]
    fn encodes_a_query() {
        assert_eq!(encode("rust 1.99 & tokio/axum"), "rust+1.99+%26+tokio%2Faxum");
    }

    #[test]
    fn numbers_results_for_the_model() {
        let results = [SearchResult {
            title: "Announcing Rust 1.99.0".to_string(),
            url: "https://blog.rust-lang.org/".to_string(),
            snippet: "The Rust team is happy".to_string(),
        }];
        assert_eq!(
            as_tool_text(&results),
            "[1] Announcing Rust 1.99.0\nhttps://blog.rust-lang.org/\nThe Rust team is happy"
        );
        assert_eq!(as_tool_text(&[]), "No results.");
    }

    #[test]
    fn keeps_six_results_with_addresses_and_short_snippets() -> Result<(), AgentError> {
        let long = "x".repeat(MAX_SNIPPET + 50);
        let mut results: Vec<Value> = (0..8)
            .map(|n| serde_json::json!({"title": format!(" t{n} "), "url": format!("https://e/{n}"), "content": long}))
            .collect();
        results.insert(0, serde_json::json!({"title": "no address"}));
        let server = FakeServer::start(vec![json_response(&serde_json::json!({ "results": results }).to_string())])?;
        let found = search(&Endpoint::new(server.address(), None), "rust tokio")?;
        let request = server.requests()?;
        assert!(request[0].starts_with("GET /search?q=rust+tokio&format=json HTTP/1.1"));
        assert_eq!(found.len(), MAX_RESULTS);
        assert_eq!((found[0].title.as_str(), found[0].url.as_str()), ("t0", "https://e/0"));
        assert_eq!(found[0].snippet.chars().count(), MAX_SNIPPET);
        Ok(())
    }

    #[test]
    fn a_searxng_error_is_reported() -> Result<(), AgentError> {
        let server = FakeServer::start(vec!["HTTP/1.1 503 Busy\r\n\r\n".to_string()])?;
        let outcome = search(&Endpoint::new(server.address(), None), "q");
        server.requests()?;
        assert!(outcome.is_err_and(|error| error.to_string() == "upstream: search returned 503"));
        Ok(())
    }
}
