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

/// How recent a search is asked to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeRange {
    /// The last day.
    Day,
    /// The last week.
    Week,
    /// The last month.
    Month,
    /// The last year.
    Year,
}

impl TimeRange {
    /// The range `name` names, case-insensitively; `None` when it names none.
    #[must_use]
    pub fn of(name: &str) -> Option<TimeRange> {
        match name.trim().to_ascii_lowercase().as_str() {
            "day" | "today" | "24h" => Some(TimeRange::Day),
            "week" | "7d" => Some(TimeRange::Week),
            "month" | "30d" => Some(TimeRange::Month),
            "year" | "12m" => Some(TimeRange::Year),
            _ => None,
        }
    }

    /// The value SearXNG expects.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TimeRange::Day => "day",
            TimeRange::Week => "week",
            TimeRange::Month => "month",
            TimeRange::Year => "year",
        }
    }
}

/// One search result, as the model and the page see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// The page title.
    pub title: String,
    /// The page address.
    pub url: String,
    /// The text the search engine shows under the title.
    pub snippet: String,
    /// The date the engine gave for the page, when it gave one.
    pub published: Option<String>,
}

/// SearXNG's JSON answer, as far as this server reads it.
#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    results: Vec<Found>,
    /// The engines that did not answer, as `[name, reason]` pairs.
    #[serde(default)]
    unresponsive_engines: Vec<(String, Option<String>)>,
}

/// What one query answered: the results, and the engines that failed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hits {
    /// The results, best first, at most [`MAX_RESULTS`].
    pub results: Vec<SearchResult>,
    /// The engines that did not answer, as `name (reason)`.
    pub down: Vec<String>,
}

/// One result in SearXNG's answer; any field may be missing or `null`.
#[derive(Deserialize)]
struct Found {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
    #[serde(rename = "publishedDate")]
    published: Option<String>,
}

/// Searches `query` on the SearXNG instance at `searxng`, keeping results no
/// older than `range` when one is given.
///
/// An empty [`Hits::results`] with a non-empty [`Hits::down`] is not "nothing
/// exists": it is the engines saying no, and the model is told so.
///
/// # Errors
///
/// `AgentError::Upstream` when SearXNG can't be reached or answers with an
/// error; `AgentError::Json` when its answer isn't JSON.
pub fn search(searxng: &Endpoint, query: &str, range: Option<TimeRange>) -> Outcome<Hits> {
    let when = range.map_or(String::new(), |range| {
        format!("&time_range={}", range.as_str())
    });
    let response = searxng.get(&format!("{SEARCH}?q={}&format=json{when}", encode(query)))?;

    if !response.is_ok() {
        return Err(AgentError::Upstream(format!(
            "search returned {}",
            response.status
        )));
    }

    let answer: Answer = serde_json::from_str(&response.text()?)?;
    Ok(Hits {
        results: answer
            .results
            .into_iter()
            .filter_map(Found::into_result)
            .take(MAX_RESULTS)
            .collect(),
        down: answer
            .unresponsive_engines
            .into_iter()
            .map(|(engine, reason)| match reason {
                Some(reason) => format!("{engine} ({reason})"),
                None => engine,
            })
            .collect(),
    })
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
            published: self
                .published
                .as_deref()
                .map(str::trim)
                .filter(|date| !date.is_empty())
                .map(ToString::to_string),
        })
    }
}

/// One result as the model reads it, under `number`, the number the answer
/// cites it by.
#[must_use]
pub fn as_entry(result: &SearchResult, number: usize) -> String {
    let date = result
        .published
        .as_deref()
        .map_or(String::new(), |date| format!("published: {date}\n"));
    format!(
        "[{number}] {}\n{}\n{date}{}",
        result.title, result.url, result.snippet
    )
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
    fn a_result_is_numbered_for_the_model_with_its_date() {
        let dated = SearchResult {
            title: "Announcing Rust 1.99.0".to_string(),
            url: "https://blog.rust-lang.org/".to_string(),
            snippet: "The Rust team is happy".to_string(),
            published: Some("2026-10-06T00:00:00".to_string()),
        };
        let plain = SearchResult {
            title: "A job".to_string(),
            url: "https://jobs.example/1".to_string(),
            snippet: "hiring".to_string(),
            published: None,
        };
        assert_eq!(
            as_entry(&dated, 7),
            "[7] Announcing Rust 1.99.0\nhttps://blog.rust-lang.org/\npublished: 2026-10-06T00:00:00\nThe Rust team is happy"
        );
        assert_eq!(
            as_entry(&plain, 2),
            "[2] A job\nhttps://jobs.example/1\nhiring"
        );
    }

    #[test]
    fn time_ranges_are_understood_and_sent() {
        assert_eq!(TimeRange::of(" Day "), Some(TimeRange::Day));
        assert_eq!(TimeRange::of("TODAY"), Some(TimeRange::Day));
        assert_eq!(TimeRange::of("7d"), Some(TimeRange::Week));
        assert_eq!(TimeRange::of("month"), Some(TimeRange::Month));
        assert_eq!(TimeRange::of("12m"), Some(TimeRange::Year));
        assert_eq!(TimeRange::of("forever"), None);
        assert_eq!(TimeRange::Day.as_str(), "day");
        assert_eq!(TimeRange::Week.as_str(), "week");
        assert_eq!(TimeRange::Month.as_str(), "month");
        assert_eq!(TimeRange::Year.as_str(), "year");
    }

    #[test]
    fn keeps_six_results_with_addresses_and_short_snippets() -> Outcome {
        let long = "x".repeat(MAX_SNIPPET + 50);
        let mut results: Vec<_> = (0..8)
            .map(|n| json!({"title": format!(" t{n} "), "url": format!("https://e/{n}"), "content": long, "publishedDate": " 2026-10-06 " }))
            .collect();
        results.insert(
            0,
            json!({"title": "no address", "url": null, "publishedDate": "  "}),
        );
        let answer = json!({ "results": results }).to_string();
        let server = FakeServer::start(vec![json_response(&answer)])?;
        let found = search(&Endpoint::new(server.address(), None), "rust tokio", Some(TimeRange::Day))?;
        let found = found.results;
        let request = server.requests()?;
        assert!(
            request[0].starts_with("GET /search?q=rust+tokio&format=json&time_range=day HTTP/1.1")
        );
        assert_eq!(found.len(), MAX_RESULTS);
        assert_eq!(
            (found[0].title.as_str(), found[0].url.as_str()),
            ("t0", "https://e/0")
        );
        assert_eq!(found[0].snippet.chars().count(), MAX_SNIPPET);
        assert_eq!(found[0].published.as_deref(), Some("2026-10-06"));
        Ok(())
    }

    #[test]
    fn an_answer_without_results_is_empty() -> Outcome {
        let server = FakeServer::start(vec![json_response("{ }")])?;
        let found = search(&Endpoint::new(server.address(), None), "q", None)?;
        server.requests()?;
        assert_eq!(found, Hits::default());
        Ok(())
    }

    #[test]
    fn a_searxng_error_is_reported() -> Outcome {
        let server = FakeServer::start(vec!["HTTP/1.1 503 Busy\r\n\r\n".to_string()])?;
        let outcome = search(&Endpoint::new(server.address(), None), "q", None);
        server.requests()?;
        assert!(outcome.is_err_and(|error| error.to_string() == "upstream: search returned 503"));
        Ok(())
    }

    #[test]
    fn engines_that_did_not_answer_are_reported() -> Outcome {
        let answer = json!({
            "results": [{ "title": "one", "url": "https://e/1" }],
            "unresponsive_engines": [
                ["duckduckgo", "CAPTCHA"],
                ["brave", "Suspended: too many requests"],
                ["wikipedia", null]
            ]
        })
        .to_string();
        let server = FakeServer::start(vec![json_response(&answer)])?;
        let hits = search(&Endpoint::new(server.address(), None), "q", None)?;
        server.requests()?;
        assert_eq!(hits.results.len(), 1);
        assert_eq!(
            hits.down,
            [
                "duckduckgo (CAPTCHA)".to_string(),
                "brave (Suspended: too many requests)".to_string(),
                "wikipedia".to_string()
            ]
        );
        Ok(())
    }
}
