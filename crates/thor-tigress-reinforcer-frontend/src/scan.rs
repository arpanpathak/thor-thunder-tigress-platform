//! A first pass over the corpus that suggests slop flags.
//!
//! The phrases come from `anti_ai_slop.md`, the list the project already keeps,
//! plus one rule for source markup that leaked into an answer. The two examples
//! flagged by hand were both caught by a rule here, which is the point: a person
//! should confirm a decision, not search for it.
//!
//! Suggestions are written to `labels/auto_flags.jsonl` and never to
//! `labels/slop_flags.jsonl`. That file holds a person's decisions, and the
//! generator removes whichever examples it names, so a machine guess must not
//! be able to delete training data on its own.

use std::{collections::BTreeMap, fs, io, path::Path};

use serde::Deserialize;

use crate::{error::ReviewError, index::Index};

/// One rule: what it is called, the flag category it maps to, and the phrases
/// that trigger it.
pub struct Rule {
    /// The rule name, for the report and for accepting a suggestion.
    pub name: &'static str,
    /// The slop category from [`crate::flags::CATEGORIES`].
    pub category: &'static str,
    /// The phrases to look for, lowercased.
    pub phrases: &'static [&'static str],
}

/// Markup from a book source that ended up inside an answer. Learned as output,
/// it teaches the model to emit HTML.
const SOURCE_MARKUP: [&str; 12] = [
    "<span", "<div", "<figure", "<figcaption", "<img", "<table", "<br", "</p",
    "</div", "&amp;", "&lt;", "&gt;",
];

/// The rules, in the order the report lists them.
pub const RULES: [Rule; 8] = [
    Rule {
        name: "source_markup",
        category: "other",
        phrases: &SOURCE_MARKUP,
    },
    Rule {
        name: "flattery_filler_opener",
        category: "flattery_filler_opener",
        phrases: &[
            "great question",
            "excellent question",
            "you're absolutely right",
            "you are absolutely right",
            "i'd be happy to help",
            "i would be happy to help",
            "happy to help",
            "what a fascinating",
            "that's a clean",
            "nicely done",
        ],
    },
    Rule {
        name: "empty_depth_words",
        category: "empty_depth_words",
        phrases: &[
            "rich tapestry",
            "delve into",
            "unlock the full potential",
            "a testament to",
            "at the intersection of",
            "in today's fast-paced world",
            "nuanced interplay",
            "navigate the complex landscape",
            "navigate the landscape",
        ],
    },
    Rule {
        name: "fake_importance",
        category: "fake_importance",
        phrases: &[
            "this is where it really matters",
            "makes all the difference",
            "earns its place",
            "game-changer",
            "the real story here",
            "more than a tool",
            "speaks volumes",
            "the real currency",
            "says the most with the fewest words",
            "it's not just",
        ],
    },
    Rule {
        name: "dramatic_setup",
        category: "dramatic_setup",
        phrases: &[
            "here's the thing",
            "let's be honest",
            "the truth is",
            "but here's where it gets interesting",
            "let that sink in",
            "spoiler:",
        ],
    },
    Rule {
        name: "fake_balance_hedging",
        category: "fake_balance_hedging",
        phrases: &[
            "it's worth noting",
            "no one-size-fits-all",
            "one size fits all",
            "ultimately, it depends",
            "both approaches have their merits",
            "it's important to consider",
        ],
    },
    Rule {
        name: "wrap_up_repeat",
        category: "wrap_up_repeat",
        phrases: &[
            "in summary",
            "in conclusion",
            "at the end of the day",
            "the bottom line",
            "hope this helps",
            "let me know if you'd like",
        ],
    },
    Rule {
        name: "rhythm_trick",
        category: "rhythm_trick",
        phrases: &["no fluff", "not because", "simple. powerful", "no filler"],
    },
];

/// The fewest em dashes in one answer before it is worth a look. One is normal
/// English; two in a short answer is the habit the project keeps seeing.
const EM_DASH_LIMIT: usize = 2;

/// One example a rule matched, in one field.
pub struct Suggestion {
    /// The example id.
    pub id: String,
    /// Which input the example came from.
    pub source: String,
    /// Which rule matched.
    pub rule: String,
    /// The flag category the rule maps to.
    pub category: String,
    /// `instruction` or `response`.
    pub field: String,
    /// The text that matched.
    pub matched: String,
}

impl Suggestion {
    /// The suggestion as one line of `auto_flags.jsonl`.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"id\":{},\"source\":{},\"rule\":{},\"category\":{},\"field\":{},\"match\":{}}}",
            quote(&self.id),
            quote(&self.source),
            quote(&self.rule),
            quote(&self.category),
            quote(&self.field),
            quote(&self.matched)
        )
    }
}

/// Wraps a string as a JSON string, escaping what has to be escaped.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control.is_control() => out.push(' '),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Runs every rule over every record.
///
/// Only the first match of each rule in each field is kept, so one paragraph of
/// slop produces one suggestion rather than twenty.
pub fn scan(index: &Index) -> Result<Vec<Suggestion>, ReviewError> {
    let mut suggestions = Vec::new();
    for (position, entry) in index.entries().enumerate() {
        let record = index.record(position)?;
        let instruction = field(&record, "instruction");
        let response = field(&record, "response");
        for (field_name, text) in [("instruction", &instruction), ("response", &response)] {
            let lowered = text.to_lowercase();
            for rule in &RULES {
                let hit = rule
                    .phrases
                    .iter()
                    .find(|phrase| lowered.contains(*phrase));
                if let Some(phrase) = hit {
                    suggestions.push(Suggestion {
                        id: entry.id.clone(),
                        source: index.source_of(entry).to_string(),
                        rule: rule.name.to_string(),
                        category: rule.category.to_string(),
                        field: field_name.to_string(),
                        matched: (*phrase).to_string(),
                    });
                }
            }
            if field_name == "response" {
                let dashes = text.matches('—').count();
                if dashes >= EM_DASH_LIMIT {
                    suggestions.push(Suggestion {
                        id: entry.id.clone(),
                        source: index.source_of(entry).to_string(),
                        rule: "em_dash_habit".to_string(),
                        category: "rhythm_trick".to_string(),
                        field: field_name.to_string(),
                        matched: format!("{dashes} em dashes"),
                    });
                }
            }
        }
    }
    Ok(suggestions)
}

/// Reads one string field out of a record line without a full deserialize.
fn field(record: &str, name: &str) -> String {
    let needle = format!("\"{name}\":\"");
    let start = match record.find(&needle) {
        Some(start) => start + needle.len(),
        None => return String::new(),
    };
    let mut out = String::new();
    let mut escaped = false;
    for character in record[start..].chars() {
        if escaped {
            out.push(match character {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '"' => break,
            other => out.push(other),
        }
    }
    out
}

/// How many suggestions each rule produced.
pub fn counts(suggestions: &[Suggestion]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for suggestion in suggestions {
        *counts.entry(suggestion.rule.clone()).or_insert(0) += 1;
    }
    counts
}

/// How many distinct examples were touched.
pub fn examples(suggestions: &[Suggestion]) -> usize {
    let ids: std::collections::BTreeSet<&str> =
        suggestions.iter().map(|s| s.id.as_str()).collect();
    ids.len()
}

/// Writes the suggestions, one JSON object per line.
pub fn write(path: &Path, suggestions: &[Suggestion]) -> Result<(), ReviewError> {
    let mut text = String::new();
    for suggestion in suggestions {
        text.push_str(&suggestion.to_json());
        text.push('\n');
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(ReviewError::io(parent))?;
    }
    fs::write(path, text).map_err(ReviewError::io(path))
}

/// One line of `labels/auto_flags.jsonl`, as [`write`] wrote it.
#[derive(Debug, Clone, Deserialize)]
pub struct SuggestionLine {
    /// The example id.
    pub id: String,
    /// Which input the example came from.
    pub source: String,
    /// Which rule matched.
    pub rule: String,
    /// The flag category the rule maps to.
    pub category: String,
    /// `instruction` or `response`.
    pub field: String,
    /// The text that matched.
    #[serde(rename = "match")]
    pub matched: String,
}

/// Reads the suggestions a previous `scan` wrote. A missing file means none.
pub fn read_suggestions(path: &Path) -> Result<Vec<SuggestionLine>, ReviewError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(ReviewError::io(path)(error)),
    };
    let mut suggestions = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        suggestions.push(serde_json::from_str(line).map_err(ReviewError::json(path, index + 1))?);
    }
    Ok(suggestions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_flattery_opener() {
        let lowered = "great question! here is the answer".to_lowercase();
        assert!(
            RULES
                .iter()
                .filter(|rule| rule.name == "flattery_filler_opener")
                .flat_map(|rule| rule.phrases.iter())
                .any(|phrase| lowered.contains(phrase))
        );
    }

    #[test]
    fn finds_leaked_source_markup() {
        let response = "<span class=\"caption\">Table B-1: Operators</span>";
        assert!(SOURCE_MARKUP.iter().any(|marker| response.contains(marker)));
    }

    #[test]
    fn reads_a_field_out_of_a_record() {
        let record = r#"{"id":"a","instruction":"What is \n a page?","response":"A block."}"#;
        assert_eq!(field(record, "response"), "A block.");
        assert!(field(record, "instruction").contains("a page?"));
    }

    #[test]
    fn quotes_a_string_for_json() {
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("a\nb"), "\"a\\nb\"");
    }
}
