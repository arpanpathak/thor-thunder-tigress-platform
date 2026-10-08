//! The bookkeeping behind a research answer: how a question is widened into
//! searches, and which sources have been seen.
//!
//! A job hunt is not one search. It is a posting, the company behind it, the
//! recruiter who wrote it, and often the hiring manager's own post. [`Kind`]
//! turns one query into the few queries that find those, and [`Ledger`] gives
//! every source a number that stays with it, so the answer can cite `[3]`
//! whether the source was found in the first search or the ninth.

use crate::search::SearchResult;

/// The most queries one `web_search` call may run.
pub const MAX_QUERIES_PER_CALL: usize = 4;

/// The most searches one answer may run. A hunt with sub-questions and the
/// people behind them needs room; the point of the number is that it ends.
pub const MAX_SEARCHES: usize = 16;

/// The most pages one answer may read.
pub const MAX_READS: usize = 12;

/// What a search is looking for, which decides how its query is widened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Anything: the query is used as it is.
    General,
    /// A posting: the query is widened towards job pages.
    Jobs,
    /// The people behind a posting: recruiters and hiring managers.
    People,
}

impl Kind {
    /// Every kind, in the order the model is told about them.
    pub const ALL: [Kind; 3] = [Kind::General, Kind::Jobs, Kind::People];

    /// The kind `name` names; anything else is [`Kind::General`].
    #[must_use]
    pub fn of(name: Option<&str>) -> Kind {
        match name.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("jobs" | "job" | "hiring" | "postings") => Kind::Jobs,
            Some("people" | "recruiter" | "recruiters" | "hiring-manager") => Kind::People,
            _ => Kind::General,
        }
    }

    /// The value the model sends and reads back.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::General => "general",
            Kind::Jobs => "jobs",
            Kind::People => "people",
        }
    }

    /// The queries `base` becomes: itself for [`Kind::General`], and, for the
    /// other two, itself followed by each angle that it does not already cover.
    /// A base that already says "hiring" does not need "hiring" again, so the
    /// widened list stays short and the search budget lasts.
    #[must_use]
    pub fn widen(self, base: &str) -> Vec<String> {
        match self {
            Kind::General => vec![base.to_string()],
            Kind::Jobs => angles(
                base,
                &[
                    ("job posting", "job"),
                    ("careers", "career"),
                    ("linkedin jobs", "linkedin"),
                ],
            ),
            Kind::People => angles(
                base,
                &[
                    ("recruiter", "recruiter"),
                    ("\"hiring manager\"", "hiring manager"),
                    ("\"we are hiring\"", "we are hiring"),
                    ("site:linkedin.com", "linkedin"),
                ],
            ),
        }
    }
}

/// `base`, then each `(angle, key)` as `base angle` when `base` does not
/// already mention `key`. The base comes first so the model's own wording is
/// always searched.
fn angles(base: &str, pairs: &[(&str, &str)]) -> Vec<String> {
    let lower = base.to_ascii_lowercase();
    let mut queries = vec![base.to_string()];
    for (angle, key) in pairs {
        if queries.len() == MAX_QUERIES_PER_CALL {
            break;
        }
        if !lower.contains(&key.to_ascii_lowercase()) {
            queries.push(format!("{base} {angle}"));
        }
    }
    queries
}

/// One source, with the number the answer cites it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The number, from 1, in the order sources were first seen.
    pub number: usize,
    /// The page title, or its address when the engine gave no title.
    pub title: String,
    /// The page address.
    pub url: String,
    /// The date the engine gave, when it gave one.
    pub published: Option<String>,
}

/// Every source a research answer has seen, and what it has spent so far.
#[derive(Debug, Default)]
pub struct Ledger {
    entries: Vec<Entry>,
    searches: usize,
    reads: usize,
}

impl Ledger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Ledger::default()
    }

    /// The number an address was given, when it has been seen before.
    #[must_use]
    pub fn number_of(&self, url: &str) -> Option<usize> {
        self.entries
            .iter()
            .find(|entry| entry.url == url)
            .map(|entry| entry.number)
    }

    /// How many sources have been seen.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no source has been seen.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many searches have run.
    #[must_use]
    pub fn searches(&self) -> usize {
        self.searches
    }

    /// How many pages have been read.
    #[must_use]
    pub fn reads(&self) -> usize {
        self.reads
    }

    /// Adds a search result and returns its number. An address already seen
    /// keeps the number it was given the first time.
    pub fn add(&mut self, result: &SearchResult) -> usize {
        if let Some(number) = self.number_of(&result.url) {
            return number;
        }
        let number = self.entries.len() + 1;
        self.entries.push(Entry {
            number,
            title: if result.title.is_empty() {
                result.url.clone()
            } else {
                result.title.clone()
            },
            url: result.url.clone(),
            published: result.published.clone(),
        });
        number
    }

    /// Adds a page that was read. An address already seen keeps its number.
    pub fn add_page(&mut self, url: &str, title: &str) -> usize {
        if let Some(number) = self.number_of(url) {
            return number;
        }
        let number = self.entries.len() + 1;
        self.entries.push(Entry {
            number,
            title: if title.is_empty() {
                url.to_string()
            } else {
                title.to_string()
            },
            url: url.to_string(),
            published: None,
        });
        number
    }

    /// Counts one search.
    pub fn count_search(&mut self) {
        self.searches += 1;
    }

    /// How many of `budget` searches are left.
    #[must_use]
    pub fn searches_left(&self, budget: usize) -> usize {
        budget.saturating_sub(self.searches)
    }

    /// How many of `budget` page reads are left.
    #[must_use]
    pub fn reads_left(&self, budget: usize) -> usize {
        budget.saturating_sub(self.reads)
    }

    /// Counts pages read.
    pub fn count_reads(&mut self, pages: usize) {
        self.reads += pages;
    }

    /// A compact list of the first `limit` sources, as the model reads them.
    #[must_use]
    pub fn list(&self, limit: usize) -> String {
        self.entries
            .iter()
            .take(limit)
            .map(Entry::as_line)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Entry {
    /// `[3] Title — url`, the shape the answer cites.
    #[must_use]
    pub fn as_line(&self) -> String {
        format!("[{}] {} — {}", self.number, self.title, self.url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(title: &str, url: &str) -> SearchResult {
        SearchResult {
            title: title.to_string(),
            url: url.to_string(),
            snippet: String::new(),
            published: None,
        }
    }

    #[test]
    fn kinds_are_read_from_what_the_model_sends() {
        assert_eq!(Kind::of(None), Kind::General);
        assert_eq!(Kind::of(Some(" Jobs ")), Kind::Jobs);
        assert_eq!(Kind::of(Some("recruiters")), Kind::People);
        assert_eq!(Kind::of(Some("weather")), Kind::General);
        assert_eq!(Kind::General.as_str(), "general");
        assert_eq!(Kind::Jobs.as_str(), "jobs");
        assert_eq!(Kind::People.as_str(), "people");
        assert_eq!(Kind::ALL.len(), 3);
    }

    #[test]
    fn a_general_query_is_left_alone() {
        assert_eq!(Kind::General.widen("rust 1.99"), ["rust 1.99"]);
    }

    #[test]
    fn a_job_hunt_keeps_the_query_and_adds_the_posting_angles() {
        let queries = Kind::Jobs.widen("senior rust engineer");
        assert_eq!(queries.len(), MAX_QUERIES_PER_CALL);
        assert_eq!(queries[0], "senior rust engineer");
        assert_eq!(queries[1], "senior rust engineer job posting");
        assert_eq!(queries[2], "senior rust engineer careers");
        assert_eq!(queries[3], "senior rust engineer linkedin jobs");
    }

    #[test]
    fn a_people_hunt_keeps_the_query_and_adds_the_people_angles() {
        let queries = Kind::People.widen("synthires rust");
        assert_eq!(queries.len(), MAX_QUERIES_PER_CALL);
        assert_eq!(queries[0], "synthires rust");
        assert_eq!(queries[1], "synthires rust recruiter");
        assert!(queries[2].contains("\"hiring manager\""));
        assert!(queries[3].contains("\"we are hiring\""));
    }

    #[test]
    fn an_angle_the_query_already_covers_is_left_out() {
        let queries = Kind::People.widen("synthires rust hiring manager");
        assert_eq!(queries[0], "synthires rust hiring manager");
        assert!(queries.contains(&"synthires rust hiring manager recruiter".to_string()));
        assert!(
            queries
                .iter()
                .all(|query| !query.contains("\"hiring manager\"")),
            "{queries:?}"
        );

        let jobs = Kind::Jobs.widen("rust job careers linkedin");
        assert_eq!(jobs, ["rust job careers linkedin"]);
    }

    #[test]
    fn every_source_gets_one_number_and_keeps_it() {
        let mut ledger = Ledger::new();
        assert_eq!(ledger.add(&result("A", "https://a/")), 1);
        assert_eq!(ledger.add(&result("B", "https://b/")), 2);
        assert_eq!(ledger.add(&result("A again", "https://a/")), 1);
        assert_eq!(ledger.add_page("https://c/", "C"), 3);
        assert_eq!(ledger.add_page("https://a/", "A"), 1);
        assert_eq!(ledger.len(), 3);
        assert!(!ledger.is_empty());
        assert_eq!(ledger.number_of("https://b/"), Some(2));
        assert_eq!(ledger.number_of("https://z/"), None);
        assert_eq!(ledger.list(1), "[1] A — https://a/");
    }

    #[test]
    fn a_missing_title_falls_back_to_the_address() {
        let mut ledger = Ledger::new();
        assert_eq!(ledger.add(&result("", "https://a/")), 1);
        assert_eq!(ledger.add_page("https://b/", ""), 2);
        assert_eq!(
            ledger.list(2),
            "[1] https://a/ — https://a/\n[2] https://b/ — https://b/"
        );
    }

    #[test]
    fn budgets_are_counted_and_run_out() {
        let mut ledger = Ledger::new();
        assert_eq!(ledger.searches_left(MAX_SEARCHES), MAX_SEARCHES);
        assert_eq!(ledger.reads_left(MAX_READS), MAX_READS);
        ledger.count_search();
        ledger.count_reads(3);
        assert_eq!(ledger.searches(), 1);
        assert_eq!(ledger.reads(), 3);
        assert_eq!(ledger.searches_left(MAX_SEARCHES), MAX_SEARCHES - 1);
        assert_eq!(ledger.reads_left(MAX_READS), MAX_READS - 3);
        assert_eq!(ledger.reads_left(2), 0);
        assert_eq!(ledger.searches_left(0), 0);
    }

    #[test]
    fn the_list_is_numbered_and_capped() {
        let mut ledger = Ledger::new();
        ledger.add(&result("A", "https://a/"));
        ledger.add(&result("B", "https://b/"));
        assert_eq!(ledger.list(10), "[1] A — https://a/\n[2] B — https://b/");
        assert_eq!(ledger.list(1), "[1] A — https://a/");
        assert_eq!(Ledger::new().list(5), "");
    }
}
