//! Web search through a SearXNG instance on localhost.

use serde::Deserialize;

use crate::{
    error::{AgentError, Outcome},
    upstream::Endpoint,
};

/// How many results are given to the model per search.
const MAX_RESULTS: usize = 6;

/// The longest snippet kept per result, in characters.
const MAX_SNIPPET: usize = 400;

/// SearXNG's search path; `format=json` asks for JSON instead of a page.
const SEARCH: &str = "/search";

/// What the model reads when a search finds nothing.
const NO_RESULTS: &str = "No results.";

/// One search result, as the model and the page see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// The page title.
    pub title: String,
    /// The page address.
    pub url: String,
    /// The text the search engine shows under the title.
    pub snippet: String,
}

/// SearXNG's JSON answer, as far as this server reads it.
#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    results: Vec<Found>,
}

/// One result in SearXNG's answer; any field may be missing or `null`.
#[derive(Deserialize)]
struct Found {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
}

/// Searches `query` on the SearXNG instance at `searxng`.
///
/// # Errors
///
/// `AgentError::Upstream` when SearXNG can't be reached or answers with an
/// error; `AgentError::Json` when its answer isn't JSON.
pub fn search(searxng: &Endpoint, query: &str) -> Outcome<Vec<SearchResult>> {
    let response = searxng.get(&format!("{SEARCH}?q={}&format=json", encode(query)))?;

    if !response.is_ok() {
        return Err(AgentError::Upstream(format!(
            "search returned {}",
            response.status
        )));
    }

    let answer: Answer = serde_json::from_str(&response.text()?)?;
    Ok(answer
        .results
        .into_iter()
        .filter_map(Found::into_result)
        .take(MAX_RESULTS)
        .collect())
}

impl Found {
    /// The result with trimmed fields and a short snippet; `None` without an address.
    fn into_result(self) -> Option<SearchResult> {
        let url = self
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())?
            .to_string();
        Some(SearchResult {
            title: self.title.as_deref().unwrap_or_default().trim().to_string(),
            url,
            snippet: self
                .content
                .as_deref()
                .unwrap_or_default()
                .trim()
                .chars()
                .take(MAX_SNIPPET)
                .collect(),
        })
    }
}

/// The results as the text the model reads: one numbered entry per result.
#[must_use]
pub fn as_tool_text(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return NO_RESULTS.to_string();
    }

    let entries: Vec<String> = results
        .iter()
        .zip(1..)
        .map(|(result, number)| {
            format!(
                "[{number}] {}\n{}\n{}",
                result.title, result.url, result.snippet
            )
        })
        .collect();
    entries.join("\n\n")
}

/// Percent-encodes `text` for a query string.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            b' ' => "+".to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, json_response};
    use serde_json::json;

    #[test]
    fn encodes_a_query() {
        assert_eq!(
            encode("rust 1.99 & tokio/axum"),
            "rust+1.99+%26+tokio%2Faxum"
        );
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
        assert_eq!(as_tool_text(&[]), NO_RESULTS);
    }

    #[test]
    fn keeps_six_results_with_addresses_and_short_snippets() -> Outcome {
        let long = "x".repeat(MAX_SNIPPET + 50);
        let mut results: Vec<_> = (0..8)
            .map(|n| json!({"title": format!(" t{n} "), "url": format!("https://e/{n}"), "content": long}))
            .collect();
        results.insert(0, json!({"title": "no address", "url": null}));
        let server = FakeServer::start(vec![json_response(
            &json!({ "results": results }).to_string(),
        )])?;
        let found = search(&Endpoint::new(server.address(), None), "rust tokio")?;
        let request = server.requests()?;
        assert!(request[0].starts_with("GET /search?q=rust+tokio&format=json HTTP/1.1"));
        assert_eq!(found.len(), MAX_RESULTS);
        assert_eq!(
            (found[0].title.as_str(), found[0].url.as_str()),
            ("t0", "https://e/0")
        );
        assert_eq!(found[0].snippet.chars().count(), MAX_SNIPPET);
        Ok(())
    }

    #[test]
    fn an_answer_without_results_is_empty() -> Outcome {
        let server = FakeServer::start(vec![json_response("{}")])?;
        let found = search(&Endpoint::new(server.address(), None), "q")?;
        server.requests()?;
        assert_eq!(found, []);
        Ok(())
    }

    #[test]
    fn a_searxng_error_is_reported() -> Outcome {
        let server = FakeServer::start(vec!["HTTP/1.1 503 Busy\r\n\r\n".to_string()])?;
        let outcome = search(&Endpoint::new(server.address(), None), "q");
        server.requests()?;
        assert!(outcome.is_err_and(|error| error.to_string() == "upstream: search returned 503"));
        Ok(())
    }
}
