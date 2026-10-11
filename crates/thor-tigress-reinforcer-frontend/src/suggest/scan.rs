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
//! be able to delete training data on its own. `apply` turns suggestions into
//! flags a person then reviews; it never touches a flag a person set.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::{
    review::{
        category::{Field, SlopCategory},
        flags::{AUTO_NOTE, Flag, FlagStore},
    },
    error::Outcome,
    datasets::{index::Index, jsonl},
};

/// One rule: what it is called, the flag category it maps to, and the
/// phrases that trigger it.
pub struct Rule {
    /// The rule name, for the report and for accepting a suggestion.
    pub name: &'static str,
    /// The slop category it suggests.
    pub category: SlopCategory,
    /// The phrases to look for, lowercased.
    pub phrases: &'static [&'static str],
}

/// Markup from a book source that ended up inside an answer. Learned as output,
/// it teaches the model to emit HTML.
const SOURCE_MARKUP: [&str; 12] = [
    "<span",
    "<div",
    "<figure",
    "<figcaption",
    "<img",
    "<table",
    "<br",
    "</p",
    "</div",
    "&amp;",
    "&lt;",
    "&gt;",
];

/// The rules, in the order the report lists them.
pub const RULES: [Rule; 8] = [
    Rule {
        name: "source_markup",
        category: SlopCategory::Other,
        phrases: &SOURCE_MARKUP,
    },
    Rule {
        name: "flattery_filler_opener",
        category: SlopCategory::FlatteryFillerOpener,
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
        category: SlopCategory::EmptyDepthWords,
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
        category: SlopCategory::FakeImportance,
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
        category: SlopCategory::DramaticSetup,
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
        category: SlopCategory::FakeBalanceHedging,
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
        category: SlopCategory::WrapUpRepeat,
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
        category: SlopCategory::RhythmTrick,
        phrases: &["no fluff", "not because", "simple. powerful", "no filler"],
    },
];

/// The rule for answers that lean on em dashes.
const EM_DASH_RULE: &str = "em_dash_habit";

/// The fewest em dashes in one answer before it is worth a look. One is normal
/// English; two in a short answer is the habit the project keeps seeing.
const EM_DASH_LIMIT: usize = 2;

/// One example a rule matched, in one field: a line of `auto_flags.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// The example id.
    pub id: String,
    /// Which input the example came from.
    pub source: String,
    /// Which rule matched.
    pub rule: String,
    /// The flag category the rule maps to.
    pub category: SlopCategory,
    /// Where the text was found.
    pub field: Field,
    /// The text that matched.
    #[serde(rename = "match")]
    pub matched: String,
}

/// What one rule found in one piece of text.
#[derive(Debug, PartialEq, Eq)]
struct Finding {
    rule: &'static str,
    category: SlopCategory,
    matched: String,
}

/// The two texts of a record the rules read.
#[derive(Deserialize)]
struct Texts {
    #[serde(default)]
    instruction: String,
    #[serde(default)]
    response: String,
}

/// Scans `training`, writes the suggestions to `out`, and prints a report.
///
/// # Errors
///
/// `ReviewError::Io` or `ReviewError::Json` for an unreadable training file,
/// `ReviewError::Io` when the suggestions can't be written.
pub fn run(training: &Path, out: &Path) -> Outcome {
    let index = Index::open(training)?;
    let suggestions = scan(&index)?;
    jsonl::write_lines(out, &suggestions)?;
    println!("scanned {} records", index.len());

    for (rule, count) in counts(&suggestions) {
        println!("  {rule:<24} {count:>6}");
    }

    println!(
        "  {:<24} {:>6} distinct examples",
        "total",
        examples(&suggestions)
    );
    println!("wrote {}", out.display());

    Ok(())
}

/// Runs every rule over every record. Only the first match of each rule in
/// each field is kept, so one paragraph of slop gives one suggestion, not twenty.
///
/// # Errors
///
/// `ReviewError::Io` or `ReviewError::Json` when a record can't be read.
pub fn scan(index: &Index) -> Outcome<Vec<Suggestion>> {
    let mut suggestions = Vec::new();

    for (position, entry) in index.entries().enumerate() {
        let texts: Texts = index.parsed(position)?;

        for (field, text) in [
            (Field::Instruction, &texts.instruction),
            (Field::Response, &texts.response),
        ] {
            suggestions.extend(findings(field, text).into_iter().map(|finding| Suggestion {
                id: entry.id.clone(),
                source: index.source_of(entry).to_string(),
                rule: finding.rule.to_string(),
                category: finding.category,
                field,
                matched: finding.matched,
            }));
        }
    }

    Ok(suggestions)
}

/// What the rules find in one field: the first phrase of each rule that
/// matches, and, in an answer, an em-dash habit.
fn findings(field: Field, text: &str) -> Vec<Finding> {
    let lowered = text.to_lowercase();
    let mut found: Vec<Finding> = RULES
        .iter()
        .filter_map(|rule| {
            let phrase = rule
                .phrases
                .iter()
                .find(|phrase| lowered.contains(*phrase))?;

            Some(Finding {
                rule: rule.name,
                category: rule.category,
                matched: (*phrase).to_string(),
            })
        })
        .collect();
    let dashes = text.matches('—').count();

    if field == Field::Response && dashes >= EM_DASH_LIMIT {
        found.push(Finding {
            rule: EM_DASH_RULE,
            category: SlopCategory::RhythmTrick,
            matched: format!("{dashes} em dashes"),
        });
    }

    found
}

/// How many suggestions each rule produced.
fn counts(suggestions: &[Suggestion]) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();

    for suggestion in suggestions {
        *counts.entry(suggestion.rule.as_str()).or_default() += 1;
    }

    counts
}

/// How many distinct examples were touched.
fn examples(suggestions: &[Suggestion]) -> usize {
    let ids: BTreeSet<&str> = suggestions
        .iter()
        .map(|suggestion| suggestion.id.as_str())
        .collect();

    ids.len()
}

/// Turns every suggestion in `suggestions` into a flag in `flags`, unless a
/// person already flagged that example, and prints how many were added.
///
/// # Errors
///
/// `ReviewError::Io` or `ReviewError::Json` for unreadable files,
/// `ReviewError::Io` when the flags can't be written.
pub fn apply(suggestions: &Path, flags: &Path) -> Outcome {
    let lines = jsonl::read_lines(suggestions)?;
    let mut store = FlagStore::open(flags)?;
    let added = apply_to(&lines, &mut store)?;
    println!("applied {added} suggestions to {}", flags.display());

    Ok(())
}

/// Adds one flag per suggested example the store doesn't flag yet, its note
/// listing every reason, and saves. Returns how many were added.
fn apply_to(suggestions: &[Suggestion], store: &mut FlagStore) -> Outcome<usize> {
    let mut reasons: BTreeMap<&str, Vec<String>> = BTreeMap::new();

    for suggestion in suggestions {
        reasons
            .entry(&suggestion.id)
            .or_default()
            .push(suggestion.reason());
    }

    let mut added = 0;

    for (id, reasons) in reasons {
        if store.get(id).is_some() {
            continue;
        }

        store.set(
            id,
            Flag {
                note: format!("{AUTO_NOTE} {}", reasons.join("; ")),
                spans: Vec::new(),
            },
        );

        added += 1;
    }

    store.save()?;

    Ok(added)
}

impl Suggestion {
    /// The suggestion in words, for a flag's note.
    fn reason(&self) -> String {
        format!(
            "{} [{}] in {} matched {:?} in {}",
            self.rule,
            self.category.name(),
            self.source,
            self.matched,
            self.field.name()
        )
    }
}

#[cfg(test)]
mod tests;
