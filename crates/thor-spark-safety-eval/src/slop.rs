//! Finds AI slop in prose, using the categories of `anti_ai_slop.md`.
//!
//! Code is blanked out before matching, so a phrase inside a code block or an
//! inline `code` span never counts. Blanking keeps every byte offset, so a hit's
//! `start` and `end` point into the original text and a reviewer's page can mark
//! the exact words.
//!
//! The test the taxonomy gives is "if you can delete the sentence and lose no
//! information, it is slop". A phrase list cannot apply that test; it finds the
//! stock phrases that almost always fail it. A clean score means "none of the
//! known phrases", not "no slop".

use std::sync::LazyLock;

use regex::{Regex, RegexBuilder};
use serde::Serialize;

/// The slop categories. The serialized names match the flag categories the
/// review page stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// "This is a game-changer."
    FakeImportance,
    /// "Here's the thing:"
    DramaticSetup,
    /// "delve into", "a rich tapestry of"
    EmptyDepthWords,
    /// "It's worth noting that..."
    FakeBalanceHedging,
    /// "Great question!"
    FlatteryFillerOpener,
    /// "In summary, ..."
    WrapUpRepeat,
    /// "Simple. Powerful. Effective.", "Not because X. Because Y."
    RhythmTrick,
}

impl Category {
    /// Every category, in taxonomy order.
    pub const ALL: [Category; 7] = [
        Category::FakeImportance,
        Category::DramaticSetup,
        Category::EmptyDepthWords,
        Category::FakeBalanceHedging,
        Category::FlatteryFillerOpener,
        Category::WrapUpRepeat,
        Category::RhythmTrick,
    ];

    /// The heading the taxonomy uses.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Category::FakeImportance => "fake importance",
            Category::DramaticSetup => "dramatic setup",
            Category::EmptyDepthWords => "empty depth words",
            Category::FakeBalanceHedging => "fake balance and hedging",
            Category::FlatteryFillerOpener => "flattery and filler openers",
            Category::WrapUpRepeat => "wrap-ups that repeat the answer",
            Category::RhythmTrick => "rhythm tricks",
        }
    }
}

/// One slop phrase found in the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    /// The category of the phrase.
    pub category: Category,
    /// The words that matched, as they appear in the text.
    pub text: String,
    /// The byte offset where the match starts.
    pub start: usize,
    /// The byte offset just past the match.
    pub end: usize,
}

/// The slop found in one piece of text.
#[derive(Debug, Clone, Serialize)]
pub struct SlopReport {
    /// Every phrase found, in text order.
    pub hits: Vec<Hit>,
    /// Em dashes in the prose. One is ordinary English; a habit of them is the
    /// "em-dash reveal" rhythm trick, so the count is reported on its own.
    pub em_dashes: usize,
    /// Words of prose, code excluded.
    pub words: usize,
}

impl SlopReport {
    /// Phrase hits plus one for an em-dash habit (two or more).
    #[must_use]
    pub fn score(&self) -> usize {
        self.hits.len() + usize::from(self.em_dashes >= EM_DASH_HABIT)
    }

    /// True when no phrase matched and there is no em-dash habit.
    #[must_use]
    pub fn clean(&self) -> bool {
        self.score() == 0
    }
}

/// The em-dash count from which a text is said to have the habit.
pub const EM_DASH_HABIT: usize = 2;

const PHRASES: [(Category, &[&str]); 6] = [
    (
        Category::FakeImportance,
        &[
            "this is where it really matters",
            "makes all the difference",
            "made all the difference",
            "earns its place",
            "game-changer",
            "game changer",
            "the real story here",
            "more than a tool",
            "speaks volumes",
            "a crucial role",
            "plays a vital role",
            "is paramount",
        ],
    ),
    (
        Category::DramaticSetup,
        &[
            "here's the thing",
            "here is the thing",
            "let's be honest",
            "the truth is",
            "here's where it gets interesting",
            "let that sink in",
            "spoiler:",
            "simpler than you'd expect",
            "here's the kicker",
            "the catch?",
        ],
    ),
    (
        Category::EmptyDepthWords,
        &[
            "rich tapestry",
            "delve into",
            "delves into",
            "delving into",
            "unlock the full potential",
            "a testament to",
            "at the intersection of",
            "in today's fast-paced world",
            "nuanced interplay",
            "navigate the complex landscape",
            "navigating the complex landscape",
            "ever-evolving landscape",
            "seamlessly integrate",
            "robust and scalable",
            "mental gymnastics",
        ],
    ),
    (
        Category::FakeBalanceHedging,
        &[
            "it's worth noting",
            "it is worth noting",
            "worth mentioning that",
            "no one-size-fits-all",
            "ultimately, it depends",
            "both approaches have their merits",
            "it's important to consider",
            "it is important to consider",
            "it's important to note",
            "it is important to note",
        ],
    ),
    (
        Category::FlatteryFillerOpener,
        &[
            "great question",
            "excellent question",
            "what a fascinating",
            "you're absolutely right",
            "you are absolutely right",
            "i'd be happy to help",
            "i would be happy to help",
            "happy to help with that",
            "certainly! ",
            "absolutely! ",
            "i completely understand",
            "i completely feel",
            "i hear you",
            "i totally get",
        ],
    ),
    (
        Category::WrapUpRepeat,
        &[
            "in summary,",
            "in conclusion,",
            "at the end of the day",
            "the bottom line:",
            "hope this helps",
            "let me know if",
            "let me know, and",
            "feel free to ask",
        ],
    ),
];

static PHRASE_PATTERNS: LazyLock<Vec<(Category, Regex)>> = LazyLock::new(|| {
    PHRASES
        .iter()
        .flat_map(|(category, phrases)| {
            phrases.iter().filter_map(|phrase| {
                let pattern = format!(
                    "{}{}{}",
                    boundary(phrase.chars().next()),
                    regex::escape(phrase).replace('\'', "['’]"),
                    boundary(phrase.chars().last())
                );
                RegexBuilder::new(&pattern)
                    .case_insensitive(true)
                    .build()
                    .ok()
                    .map(|regex| (*category, regex))
            })
        })
        .collect()
});

/// A word boundary when the phrase starts or ends with a word character, so
/// "the truth is" does not match "the truth isn't".
fn boundary(edge: Option<char>) -> &'static str {
    match edge {
        Some(character) if character.is_alphanumeric() => r"\b",
        _ => "",
    }
}

static RHYTHM_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)\bnot because\b[^.!?\n]{1,100}[.!?]\s+because\b",
        r"(?i)\b(?:it's|it’s|it is|this is|that's|that’s)\s+not\s+(?:just|only|merely)\b[^.!?\n]{1,80}[,;—–-]\s*(?:it's|it’s|it is|but)\b",
        r"\b[A-Z][a-z]+\.\s+[A-Z][a-z]+\.\s+[A-Z][a-z]+\.",
        r"(?i)\bno \w+\.\s+no \w+\.",
    ]
    .iter()
    .filter_map(|pattern| Regex::new(pattern).ok())
    .collect()
});

static FENCE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?ms)^[ \t]*(```|~~~).*?(?:^[ \t]*(```|~~~)[^\n]*$|\z)").ok());

static INLINE_CODE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"`[^`\n]+`").ok());

/// `text` with fenced blocks and inline code replaced by spaces, byte for byte.
#[must_use]
pub fn prose_only(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let patterns = [FENCE.as_ref(), INLINE_CODE.as_ref()];
    for regex in patterns.into_iter().flatten() {
        let blank_text = String::from_utf8_lossy(&bytes).into_owned();
        for found in regex.find_iter(&blank_text) {
            for byte in bytes[found.range()].iter_mut().filter(|byte| **byte != b'\n') {
                *byte = b' ';
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Finds the slop in `text`.
#[must_use]
pub fn check(text: &str) -> SlopReport {
    let prose = prose_only(text);
    let mut hits: Vec<Hit> = PHRASE_PATTERNS
        .iter()
        .flat_map(|(category, regex)| {
            regex
                .find_iter(&prose)
                .map(move |found| (*category, found.start(), found.end()))
        })
        .chain(RHYTHM_PATTERNS.iter().flat_map(|regex| {
            regex
                .find_iter(&prose)
                .map(|found| (Category::RhythmTrick, found.start(), found.end()))
        }))
        .filter_map(|(category, start, end)| {
            let found = prose.get(start..end)?;
            let start = start + found.len() - found.trim_start().len();
            Some(Hit {
                category,
                text: text.get(start..end)?.to_string(),
                start,
                end,
            })
        })
        .collect();
    hits.sort_by_key(|hit| (hit.start, hit.category));
    hits.dedup_by(|later, earlier| later.start < earlier.end);
    SlopReport {
        hits,
        em_dashes: prose.matches('—').count(),
        words: prose.split_whitespace().count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_every_category() {
        let labels: Vec<&str> = [
            Category::FakeImportance,
            Category::DramaticSetup,
            Category::EmptyDepthWords,
            Category::FakeBalanceHedging,
            Category::FlatteryFillerOpener,
            Category::WrapUpRepeat,
            Category::RhythmTrick,
        ]
        .into_iter()
        .map(Category::label)
        .collect();
        assert_eq!(labels.len(), 7);
        assert!(labels.iter().all(|label| !label.is_empty()));
    }

    fn categories(text: &str) -> Vec<Category> {
        check(text)
            .hits
            .into_iter()
            .map(|hit| hit.category)
            .collect()
    }

    #[test]
    fn finds_one_phrase_from_each_listed_category() {
        assert_eq!(categories("Great question! Let's delve into it."), vec![
            Category::FlatteryFillerOpener,
            Category::EmptyDepthWords
        ]);
        assert_eq!(categories("Here’s the thing: it works."), vec![Category::DramaticSetup]);
        assert_eq!(categories("In summary, use iterators."), vec![Category::WrapUpRepeat]);
    }

    #[test]
    fn finds_rhythm_tricks() {
        assert_eq!(categories("Simple. Powerful. Effective."), vec![Category::RhythmTrick]);
        assert_eq!(categories("Not because it is fast. Because it is clear."), vec![Category::RhythmTrick]);
        assert_eq!(categories("It's not just a parser, it's a mindset."), vec![Category::RhythmTrick]);
    }

    #[test]
    fn ignores_phrases_inside_code() {
        let text = "Run this:\n```rust\n// great question\nlet x = 1;\n```\nand `delve into` is a name.";
        assert_eq!(check(text).hits, []);
    }

    #[test]
    fn offsets_point_into_the_original_text() {
        let text = "Ok. `x` — great question — sure.";
        let report = check(text);
        let hit = report.hits.first().map(|hit| &text[hit.start..hit.end]);
        assert_eq!(hit, Some("great question"));
        assert_eq!(report.em_dashes, 2);
        assert!(!report.clean());
    }

    #[test]
    fn does_not_match_inside_a_longer_word() {
        assert_eq!(check("The game-changers list is a heading.").hits, []);
        assert_eq!(check("Antithesis: the truth isn't here.").hits, []);
    }

    #[test]
    fn plain_technical_prose_is_clean() {
        let text = "The parser reads one line at a time. A line that starts with # is skipped.";
        assert!(check(text).clean());
    }
}
