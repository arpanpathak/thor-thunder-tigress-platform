//! The slop flags a reviewer sets.
//!
//! ## Format
//!
//! `labels/slop_flags.jsonl` holds one flag per line:
//!
//! ```text
//!   {"id": "9f3c2a71d04e8b65", "note": "opens with flattery",
//!    "spans": [{"field": "response", "text": "Great question!", "category": "flattery_filler_opener"}]}
//! ```
//!
//! This is the same file the data generator reads, so a flag set here is
//! applied by the next build without any conversion step.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::error::ReviewError;

/// The slop categories of `anti_ai_slop.md`, as `(name, description)`.
pub const CATEGORIES: [(&str, &str); 8] = [
    ("fake_importance", "fake importance"),
    ("dramatic_setup", "dramatic setup before something ordinary"),
    ("empty_depth_words", "empty depth words"),
    ("fake_balance_hedging", "fake balance and hedging"),
    ("flattery_filler_opener", "flattery and filler openers"),
    ("wrap_up_repeat", "wrap-ups that repeat the answer"),
    ("rhythm_trick", "rhythm tricks"),
    ("other", "other"),
];

/// One phrase a reviewer marked, and which part of the example it was in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// `instruction` or `response`.
    pub field: String,
    /// The marked text.
    pub text: String,
    /// One of [`CATEGORIES`].
    pub category: String,
}

/// One example's flag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flag {
    /// The reviewer's note.
    #[serde(default)]
    pub note: String,
    /// The marked phrases.
    #[serde(default)]
    pub spans: Vec<Span>,
}

impl Flag {
    /// False for a flag a machine suggested and no person has touched: its
    /// note starts with `auto:` and it marks no phrase.
    pub fn is_reviewed(&self) -> bool {
        !(self.note.starts_with("auto:") && self.spans.is_empty())
    }
}

/// One line of the flags file.
#[derive(Deserialize)]
struct FlagLine {
    /// The example id.
    id: String,
    /// The reviewer's note.
    #[serde(default)]
    note: String,
    /// The marked phrases.
    #[serde(default)]
    spans: Vec<Span>,
}

/// One line of the flags file, for writing.
#[derive(Serialize)]
struct FlagLineRef<'a> {
    /// The example id.
    id: &'a str,
    /// The reviewer's note.
    note: &'a str,
    /// The marked phrases.
    spans: &'a [Span],
}

/// A phrase a reviewer marked, as [`FlagStore::phrases`] lists it.
#[derive(Debug, Clone, Serialize)]
pub struct Phrase {
    /// The phrase as it was first marked.
    pub text: String,
    /// The category it was first marked as.
    pub category: String,
    /// How many marks carry it.
    pub examples: usize,
}

/// Every flag, and the file they are kept in.
pub struct FlagStore {
    path: PathBuf,
    flags: HashMap<String, Flag>,
}

impl FlagStore {
    /// Reads the flags, treating a missing file as no flags yet.
    pub fn open(path: &Path) -> Result<Self, ReviewError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(ReviewError::io(path)(error)),
        };
        let mut flags = HashMap::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let parsed: FlagLine =
                serde_json::from_str(line).map_err(ReviewError::json(path, index + 1))?;
            flags.insert(
                parsed.id,
                Flag {
                    note: parsed.note,
                    spans: parsed.spans,
                },
            );
        }
        Ok(FlagStore {
            path: path.to_path_buf(),
            flags,
        })
    }

    /// The ids that carry a flag.
    pub fn ids(&self) -> HashSet<String> {
        self.flags.keys().cloned().collect()
    }

    /// How many examples are flagged.
    pub fn len(&self) -> usize {
        self.flags.len()
    }

    /// The flag of one example, if it has one.
    pub fn get(&self, id: &str) -> Option<&Flag> {
        self.flags.get(id)
    }

    /// Every flag, sorted by id, so a caller can look for orphans.
    pub fn all(&self) -> Vec<(String, Flag)> {
        let mut entries: Vec<(String, Flag)> = self
            .flags
            .iter()
            .map(|(id, flag)| (id.clone(), flag.clone()))
            .collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries
    }

    /// Sets or replaces one example's flag.
    pub fn set(&mut self, id: &str, flag: Flag) {
        self.flags.insert(id.to_string(), flag);
    }

    /// Removes one example's flag.
    pub fn clear(&mut self, id: &str) {
        self.flags.remove(id);
    }

    /// Every phrase a reviewer has marked, once each, with the category it was
    /// first marked as and how many examples carry it. The page highlights
    /// these in every other example, so a phrase marked once is seen everywhere.
    pub fn phrases(&self) -> Vec<Phrase> {
        let mut found: HashMap<String, Phrase> = HashMap::new();
        for span in self.flags.values().flat_map(|flag| flag.spans.iter()) {
            let text = span.text.trim();
            if text.chars().count() < 4 {
                continue;
            }
            found
                .entry(text.to_lowercase())
                .and_modify(|phrase| phrase.examples += 1)
                .or_insert_with(|| Phrase {
                    text: text.to_string(),
                    category: span.category.clone(),
                    examples: 1,
                });
        }
        let mut phrases: Vec<Phrase> = found.into_values().collect();
        phrases.sort_by(|left, right| right.examples.cmp(&left.examples).then_with(|| left.text.cmp(&right.text)));
        phrases
    }

    /// Adds `span` to the flag of `id`, creating the flag with `note` when the
    /// example has none. Returns false when the example already carries the
    /// same phrase in the same field.
    pub fn add_span(&mut self, id: &str, span: Span, note: &str) -> bool {
        let flag = self.flags.entry(id.to_string()).or_insert_with(|| Flag {
            note: note.to_string(),
            spans: Vec::new(),
        });
        let present = flag
            .spans
            .iter()
            .any(|kept| kept.field == span.field && kept.text.eq_ignore_ascii_case(&span.text));
        if !present {
            flag.spans.push(span);
        }
        !present
    }

    /// True when `category` is one of [`CATEGORIES`].
    pub fn is_known_category(category: &str) -> bool {
        CATEGORIES.iter().any(|(name, _)| *name == category)
    }

    /// Writes every flag back, through a temporary file so a crash cannot
    /// leave a half-written flag file.
    pub fn save(&self) -> Result<(), ReviewError> {
        let mut ids: Vec<&String> = self.flags.keys().collect();
        ids.sort();
        let mut text = String::new();
        for id in ids {
            let flag = self.flags.get(id).ok_or_else(|| {
                ReviewError::BadRequest(format!("flag {id} vanished while saving"))
            })?;
            let line = FlagLineRef {
                id,
                note: &flag.note,
                spans: &flag.spans,
            };
            let encoded = serde_json::to_string(&line)
                .map_err(|error| ReviewError::BadRequest(error.to_string()))?;
            text.push_str(&encoded);
            text.push('\n');
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(ReviewError::io(parent))?;
        }
        let temporary = self.path.with_extension("jsonl.tmp");
        fs::write(&temporary, text).map_err(ReviewError::io(&temporary))?;
        fs::rename(&temporary, &self.path).map_err(ReviewError::io(&self.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary path for one test.
    fn temporary(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn a_missing_file_means_no_flags() -> Result<(), ReviewError> {
        let path = temporary("flagger_missing_flags.jsonl");
        fs::remove_file(&path).ok();
        let store = FlagStore::open(&path)?;
        assert_eq!(store.len(), 0);
        Ok(())
    }

    #[test]
    fn saves_and_reads_back_a_flag() -> Result<(), ReviewError> {
        let path = temporary("flagger_flags_roundtrip.jsonl");
        fs::remove_file(&path).ok();
        let mut store = FlagStore::open(&path)?;
        store.set(
            "abc",
            Flag {
                note: "opens with flattery".to_string(),
                spans: vec![Span {
                    field: "response".to_string(),
                    text: "Great question!".to_string(),
                    category: "flattery_filler_opener".to_string(),
                }],
            },
        );
        store.save()?;
        let reopened = FlagStore::open(&path)?;
        assert_eq!(reopened.len(), 1);
        assert_eq!(
            reopened.get("abc").map(|flag| flag.spans.len()),
            Some(1)
        );
        fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn clearing_a_flag_removes_it() -> Result<(), ReviewError> {
        let path = temporary("flagger_flags_clear.jsonl");
        fs::remove_file(&path).ok();
        let mut store = FlagStore::open(&path)?;
        store.set("abc", Flag::default());
        store.clear("abc");
        store.save()?;
        let reopened = FlagStore::open(&path)?;
        assert_eq!(reopened.len(), 0);
        fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn lists_each_marked_phrase_once_with_its_count() -> Result<(), ReviewError> {
        let path = temporary("flagger_phrases.jsonl");
        fs::remove_file(&path).ok();
        let mut store = FlagStore::open(&path)?;
        let span = |text: &str| Span {
            field: "response".to_string(),
            text: text.to_string(),
            category: "dramatic_setup".to_string(),
        };
        store.add_span("a", span("Here's the thing"), "");
        store.add_span("b", span("here's the thing"), "");
        assert!(!store.add_span("b", span("HERE'S THE THING"), ""));
        let phrases = store.phrases();
        assert_eq!(phrases.len(), 1);
        assert_eq!(phrases.first().map(|phrase| phrase.examples), Some(2));
        Ok(())
    }

    #[test]
    fn knows_the_categories() {
        assert!(FlagStore::is_known_category("fake_importance"));
        assert!(!FlagStore::is_known_category("made_up"));
    }
}
