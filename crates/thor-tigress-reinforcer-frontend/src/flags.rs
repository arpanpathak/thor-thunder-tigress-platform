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
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    category::{Field, SlopCategory},
    error::Outcome,
    jsonl,
};

/// A phrase shorter than this, in characters, is too common to mark everywhere.
pub const MIN_PHRASE_CHARS: usize = 4;

/// How a flag a machine suggested begins its note.
pub const AUTO_NOTE: &str = "auto:";

/// One phrase a reviewer marked, and which part of the example it was in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// The part of the example.
    pub field: Field,
    /// The marked text.
    pub text: String,
    /// What kind of slop it is.
    pub category: SlopCategory,
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
    #[must_use]
    pub fn is_reviewed(&self) -> bool {
        !(self.note.starts_with(AUTO_NOTE) && self.spans.is_empty())
    }
}

/// One line of the flags file.
#[derive(Serialize, Deserialize)]
struct FlagLine<Id, Body> {
    id: Id,
    #[serde(flatten)]
    flag: Body,
}

/// A phrase a reviewer marked, as [`FlagStore::phrases`] lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Phrase {
    /// The phrase as it was first marked.
    pub text: String,
    /// The category it was first marked as.
    pub category: SlopCategory,
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
    ///
    /// # Errors
    ///
    /// `ReviewError::Io` or `ReviewError::Json` for an unreadable file.
    pub fn open(path: &Path) -> Outcome<Self> {
        let lines: Vec<FlagLine<String, Flag>> = jsonl::read_lines(path)?;
        Ok(FlagStore {
            path: path.to_path_buf(),
            flags: lines.into_iter().map(|line| (line.id, line.flag)).collect(),
        })
    }

    /// The ids that carry a flag.
    #[must_use]
    pub fn ids(&self) -> HashSet<String> {
        self.flags.keys().cloned().collect()
    }

    /// How many examples are flagged.
    #[must_use]
    pub fn len(&self) -> usize {
        self.flags.len()
    }

    /// The flag of one example, if it has one.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Flag> {
        self.flags.get(id)
    }

    /// Every flag with its id, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Flag)> {
        self.flags.iter().map(|(id, flag)| (id.as_str(), flag))
    }

    /// Sets or replaces one example's flag.
    pub fn set(&mut self, id: &str, flag: Flag) {
        self.flags.insert(id.to_string(), flag);
    }

    /// Removes one example's flag.
    pub fn clear(&mut self, id: &str) {
        self.flags.remove(id);
    }

    /// Every phrase a reviewer has marked, once each, with the spelling and
    /// category of its first mark (by example id) and how many marks carry it,
    /// most common first. The page highlights these in every other example, so
    /// a phrase marked once is seen everywhere.
    #[must_use]
    pub fn phrases(&self) -> Vec<Phrase> {
        let mut flags: Vec<(&str, &Flag)> = self.iter().collect();
        flags.sort_by_key(|(id, _)| *id);
        let mut found: HashMap<String, Phrase> = HashMap::new();
        for span in flags.into_iter().flat_map(|(_, flag)| &flag.spans) {
            let text = span.text.trim();
            if text.chars().count() < MIN_PHRASE_CHARS {
                continue;
            }
            found
                .entry(text.to_lowercase())
                .and_modify(|phrase| phrase.examples += 1)
                .or_insert_with(|| Phrase { text: text.to_string(), category: span.category, examples: 1 });
        }
        let mut phrases: Vec<Phrase> = found.into_values().collect();
        phrases.sort_by(|left, right| right.examples.cmp(&left.examples).then_with(|| left.text.cmp(&right.text)));
        phrases
    }

    /// Adds `span` to the flag of `id`, creating the flag with `note` when the
    /// example has none. Returns false when the example already carries the
    /// same phrase in the same field.
    pub fn add_span(&mut self, id: &str, span: Span, note: &str) -> bool {
        let flag = self
            .flags
            .entry(id.to_string())
            .or_insert_with(|| Flag { note: note.to_string(), spans: Vec::new() });
        let present = flag
            .spans
            .iter()
            .any(|kept| kept.field == span.field && kept.text.eq_ignore_ascii_case(&span.text));
        if !present {
            flag.spans.push(span);
        }
        !present
    }

    /// Writes every flag back, sorted by id.
    ///
    /// # Errors
    ///
    /// `ReviewError::Io` when the file can't be written.
    pub fn save(&self) -> Outcome {
        let mut lines: Vec<FlagLine<&str, &Flag>> = self.iter().map(|(id, flag)| FlagLine { id, flag }).collect();
        lines.sort_by_key(|line| line.id);
        jsonl::write_lines(&self.path, &lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn span(text: &str) -> Span {
        Span { field: Field::Response, text: text.to_string(), category: SlopCategory::DramaticSetup }
    }

    #[test]
    fn a_missing_file_means_no_flags() -> Outcome {
        let folder = TempDir::new()?;
        assert_eq!(FlagStore::open(&folder.path().join("flags.jsonl"))?.len(), 0);
        Ok(())
    }

    #[test]
    fn saves_and_reads_back_in_the_files_format() -> Outcome {
        let folder = TempDir::new()?;
        let path = folder.path().join("flags.jsonl");
        let mut store = FlagStore::open(&path)?;
        store.set("b", Flag { note: "opens with flattery".to_string(), spans: vec![span("Here's the thing")] });
        store.set("a", Flag::default());
        store.save()?;
        let text = std::fs::read_to_string(&path).map_err(crate::error::ReviewError::io(&path))?;
        assert_eq!(
            text,
            "{\"id\":\"a\",\"note\":\"\",\"spans\":[]}\n{\"id\":\"b\",\"note\":\"opens with flattery\",\"spans\":[{\"field\":\"response\",\"text\":\"Here's the thing\",\"category\":\"dramatic_setup\"}]}\n"
        );
        let reopened = FlagStore::open(&path)?;
        assert_eq!(reopened.get("b").map(|flag| flag.spans.len()), Some(1));
        assert_eq!(reopened.ids().len(), 2);
        Ok(())
    }

    #[test]
    fn clearing_a_flag_removes_it() -> Outcome {
        let folder = TempDir::new()?;
        let mut store = FlagStore::open(&folder.path().join("flags.jsonl"))?;
        store.set("abc", Flag::default());
        store.clear("abc");
        assert_eq!(store.get("abc"), None);
        Ok(())
    }

    #[test]
    fn lists_each_marked_phrase_once_with_its_count() -> Outcome {
        let folder = TempDir::new()?;
        let mut store = FlagStore::open(&folder.path().join("flags.jsonl"))?;
        assert!(store.add_span("a", span("Here's the thing"), ""));
        assert!(store.add_span("b", span("here's the thing"), ""));
        assert!(!store.add_span("b", span("HERE'S THE THING"), ""));
        assert!(store.add_span("c", span("ok"), ""));
        let phrases = store.phrases();
        assert_eq!(phrases, [Phrase { text: "Here's the thing".to_string(), category: SlopCategory::DramaticSetup, examples: 2 }]);
        Ok(())
    }

    #[test]
    fn a_machine_flag_is_not_reviewed_until_a_person_marks_it() {
        let auto = Flag { note: "auto: in summary".to_string(), spans: Vec::new() };
        let marked = Flag { note: "auto: in summary".to_string(), spans: vec![span("In summary")] };
        assert!(!auto.is_reviewed());
        assert!(marked.is_reviewed());
        assert!(Flag::default().is_reviewed());
    }
}
