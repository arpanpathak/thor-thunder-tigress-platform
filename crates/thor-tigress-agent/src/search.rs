//! Web search through a SearXNG instance on localhost.

use serde_json::Value;

use crate::{error::AgentError, http};

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

/// Searches `query` on the SearXNG instance at `address` (`host:port`).
pub fn search(address: &str, query: &str) -> Result<Vec<SearchResult>, AgentError> {
    let path = format!("/search?q={}&format=json", encode(query));
    let response = http::call(address, "GET", &path, None, None)?;
    if response.status != 200 {
        return Err(AgentError::Upstream(format!("search returned {}", response.status)));
    }
    let parsed: Value = serde_json::from_str(&response.text()?)?;
    let results = parsed
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .filter_map(|result| {
                    let text = |name: &str| result.get(name).and_then(Value::as_str).unwrap_or_default();
                    let url = text("url");
                    (!url.is_empty()).then(|| SearchResult {
                        title: text("title").trim().to_string(),
                        url: url.to_string(),
                        snippet: text("content").trim().chars().take(MAX_SNIPPET).collect(),
                    })
                })
                .take(MAX_RESULTS)
                .collect()
        })
        .unwrap_or_default();
    Ok(results)
}

/// The results as the text the model reads: one numbered entry per result.
pub fn as_tool_text(results: &[SearchResult]) -> String {
    match results.is_empty() {
        true => "No results.".to_string(),
        false => results
            .iter()
            .enumerate()
            .map(|(position, result)| {
                format!("[{}] {}\n{}\n{}", position + 1, result.title, result.url, result.snippet)
            })
            .collect::<Vec<String>>()
            .join("\n\n"),
    }
}

/// Percent-encodes `text` for a query string.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            b' ' => "+".to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
