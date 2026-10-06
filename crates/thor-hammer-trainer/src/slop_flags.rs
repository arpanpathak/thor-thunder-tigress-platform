//! Slop flags set by a reviewer on the review page.
//!
//! ## Format
//!
//! `labels/slop_flags.jsonl` holds one flag per example:
//!
//! ```text
//!   {"id": "9f3c2a71d04e8b65",
//!    "note": "opens with flattery",
//!    "spans": [{"field": "response", "text": "Great question!", "category": "flattery_filler_opener"}]}
//! ```
//!
//! A flag marks the whole example as slop. `spans` optionally marks the exact
//! sentences and names their category from `anti_ai_slop.md`. A span stores its
//! text rather than character offsets, so it stays valid when whitespace in
//! the source changes.
//!
//! `id` is [`Example::id`], so a flag survives rebuilds as long as the example's
//! text does not change. The file lives in `labels/`, not in the generated
//! `data/` folder, because it is human work and is committed.
//!
//! A flagged example is removed from `train.jsonl` and written to `slop.jsonl`
//! with its note and spans.

use std::{collections::HashMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    error::DataError,
    example::{Example, SkipReason},
};

/// The slop categories of `anti_ai_slop.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlopCategory {
    /// "This is where it really matters."
    FakeImportance,
    /// "Here's the thing:" before an ordinary statement.
    DramaticSetup,
    /// "delve into", "a rich tapestry of".
    EmptyDepthWords,
    /// "It's worth noting that...", "Both approaches have their merits."
    FakeBalanceHedging,
    /// "Great question!", "I'd be happy to help with that!"
    FlatteryFillerOpener,
    /// "In summary, ...", "Hope this helps!"
    WrapUpRepeat,
    /// Triplets, one-word fragments, em-dash reveals.
    RhythmTrick,
    /// Slop that fits none of the categories above.
    Other,
}

impl SlopCategory {
    /// Every category, in the order the report lists them.
    pub const ALL: [SlopCategory; 8] = [
        SlopCategory::FakeImportance,
        SlopCategory::DramaticSetup,
        SlopCategory::EmptyDepthWords,
        SlopCategory::FakeBalanceHedging,
        SlopCategory::FlatteryFillerOpener,
        SlopCategory::WrapUpRepeat,
        SlopCategory::RhythmTrick,
        SlopCategory::Other,
    ];

    /// The name used in the report, as in `anti_ai_slop.md`.
    pub fn description(self) -> &'static str {
        match self {
            SlopCategory::FakeImportance => "fake importance",
            SlopCategory::DramaticSetup => "dramatic setup before something ordinary",
            SlopCategory::EmptyDepthWords => "empty depth words",
            SlopCategory::FakeBalanceHedging => "fake balance and hedging",
            SlopCategory::FlatteryFillerOpener => "flattery and filler openers",
            SlopCategory::WrapUpRepeat => "wrap-ups that repeat the answer",
            SlopCategory::RhythmTrick => "rhythm tricks",
            SlopCategory::Other => "other",
        }
    }
}

/// Which part of an example a span was selected in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Field {
    Instruction,
    Response,
}

/// One sentence or phrase a reviewer marked as slop.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SlopSpan {
    pub field: Field,
    pub text: String,
    pub category: SlopCategory,
}

/// One line of `labels/slop_flags.jsonl`.
#[derive(Debug, Clone, Deserialize)]
pub struct SlopFlag {
    id: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    spans: Vec<SlopSpan>,
}

impl SlopFlag {
    /// False for a flag a machine suggested and no person has touched: its
    /// note starts with `auto:` and it marks no phrase. Only a reviewed flag
    /// removes an example, so a machine guess cannot delete training data.
    pub fn is_reviewed(&self) -> bool {
        !(self.note.starts_with("auto:") && self.spans.is_empty())
    }
}

/// One line of `slop.jsonl`: the flagged example with its id, note and spans.
#[derive(Serialize)]
pub struct FlaggedExample {
    id: String,
    note: String,
    pub spans: Vec<SlopSpan>,
    #[serde(flatten)]
    example: Example,
}

/// The reviewer's flags, keyed by example id. A missing file means no flags yet.
pub fn read(path: &Path) -> Result<HashMap<String, SlopFlag>, DataError> {
    let flags_jsonl = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(DataError::io(path)(error)),
    };
    let mut flags_by_id = HashMap::new();
    for line in flags_jsonl
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let flag: SlopFlag = serde_json::from_str(line)?;
        flags_by_id.insert(flag.id.clone(), flag);
    }
    Ok(flags_by_id)
}

/// A reviewer flag whose id matched no example in the build.
///
/// This is what a rebuild that changes an example's text leaves behind: the id
/// is a hash of the text, so the flag can no longer find its example. Such a
/// flag must be reported and re-pointed, never dropped without a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmatchedFlag {
    /// The id the flag carried.
    pub id: String,
    /// The reviewer's note, so the flag can be re-pointed by hand.
    pub note: String,
}

/// Splits `examples` into kept and flagged, and reports flags that matched none.
pub fn separate_flagged(
    examples: Vec<Example>,
    flags_by_id: &HashMap<String, SlopFlag>,
    skip_reasons: &mut Vec<SkipReason>,
) -> (Vec<Example>, Vec<FlaggedExample>, Vec<UnmatchedFlag>) {
    let mut kept = Vec::new();
    let mut flagged = Vec::new();
    let mut matched_ids = std::collections::HashSet::new();
    for example in examples {
        let id = example.id();
        match flags_by_id.get(&id) {
            Some(flag) if !flag.is_reviewed() => {
                matched_ids.insert(id.clone());
                kept.push(example);
            }
            Some(flag) => {
                skip_reasons.push(SkipReason::FlaggedSlop);
                matched_ids.insert(id.clone());
                flagged.push(FlaggedExample {
                    id,
                    note: flag.note.clone(),
                    spans: flag.spans.clone(),
                    example,
                });
            }
            None => kept.push(example),
        }
    }
    let mut unmatched: Vec<UnmatchedFlag> = flags_by_id
        .iter()
        .filter(|(id, flag)| !matched_ids.contains(*id) && flag.is_reviewed())
        .map(|(id, flag)| UnmatchedFlag {
            id: id.clone(),
            note: flag.note.clone(),
        })
        .collect();
    unmatched.sort_by(|left, right| left.id.cmp(&right.id));
    (kept, flagged, unmatched)
}

/// How many flagged spans fall in each category, in [`SlopCategory::ALL`] order.
pub fn span_counts(flagged: &[FlaggedExample]) -> Vec<(SlopCategory, usize)> {
    let all_spans: Vec<&SlopSpan> = flagged
        .iter()
        .flat_map(|flagged_example| &flagged_example.spans)
        .collect();
    SlopCategory::ALL
        .into_iter()
        .map(|category| {
            let count = all_spans
                .iter()
                .filter(|span| span.category == category)
                .count();
            (category, count)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flags_path_that_is_a_folder_is_an_error() {
        assert!(matches!(read(&std::env::temp_dir()), Err(DataError::Io { .. })));
    }
    use crate::example::Source;

    fn example(response: &str) -> Example {
        Example {
            instruction: "What is a page?".to_string(),
            response: response.to_string(),
            source: Source::Chat,
            origin: String::new(),
        }
    }

    #[test]
    fn a_machine_flag_nobody_reviewed_removes_nothing() -> Result<(), DataError> {
        let guessed = example("A page is a block of memory \u{2014} fixed size \u{2014} mapped by the MMU.");
        let flag_line = format!(r#"{{"id":"{}","note":"auto: em_dash_habit","spans":[]}}"#, guessed.id());
        let flag: SlopFlag = serde_json::from_str(&flag_line)?;
        let flags_by_id = HashMap::from([(guessed.id(), flag)]);
        let mut skip_reasons = Vec::new();
        let (kept, flagged, unmatched) = separate_flagged(vec![guessed.clone()], &flags_by_id, &mut skip_reasons);
        assert_eq!(kept, [guessed]);
        assert!(flagged.is_empty() && unmatched.is_empty() && skip_reasons.is_empty());
        Ok(())
    }

    #[test]
    fn flagged_examples_leave_the_training_set_with_their_spans() -> Result<(), DataError> {
        let sloppy = example("Great question! A page is a block of memory. Hope this helps!");
        let plain = example("A page is a fixed-size block of virtual memory.");
        let flag_line = format!(
            r#"{{"id":"{}","note":"opener and sign-off","spans":[{{"field":"response","text":"Great question!","category":"flattery_filler_opener"}},{{"field":"response","text":"Hope this helps!","category":"wrap_up_repeat"}}]}}"#,
            sloppy.id()
        );
        let flag: SlopFlag = serde_json::from_str(&flag_line)?;
        let flags_by_id = HashMap::from([(sloppy.id(), flag)]);
        let mut skip_reasons = Vec::new();

        let (kept, flagged, unmatched) =
            separate_flagged(vec![sloppy, plain.clone()], &flags_by_id, &mut skip_reasons);

        assert_eq!(kept, [plain]);
        assert_eq!(skip_reasons, [SkipReason::FlaggedSlop]);
        assert!(unmatched.is_empty());
        let counts = span_counts(&flagged);
        assert!(counts.contains(&(SlopCategory::FlatteryFillerOpener, 1)));
        assert!(counts.contains(&(SlopCategory::WrapUpRepeat, 1)));
        assert!(counts.contains(&(SlopCategory::RhythmTrick, 0)));
        Ok(())
    }

    #[test]
    fn flag_without_spans_is_still_a_flag() -> Result<(), DataError> {
        let flag: SlopFlag = serde_json::from_str(r#"{"id":"abc","note":"filler"}"#)?;
        assert!(flag.spans.is_empty());
        Ok(())
    }

    #[test]
    fn flag_that_matches_no_example_is_reported() -> Result<(), DataError> {
        let plain = example("A page is a fixed-size block of virtual memory.");
        let flag: SlopFlag =
            serde_json::from_str(r#"{"id":"deadbeefdeadbeef","note":"Bad training data"}"#)?;
        let flags_by_id = HashMap::from([(String::from("deadbeefdeadbeef"), flag)]);
        let mut skip_reasons = Vec::new();

        let (kept, flagged, unmatched) =
            separate_flagged(vec![plain.clone()], &flags_by_id, &mut skip_reasons);

        assert_eq!(kept, [plain]);
        assert!(flagged.is_empty());
        assert_eq!(
            unmatched,
            [UnmatchedFlag {
                id: String::from("deadbeefdeadbeef"),
                note: String::from("Bad training data"),
            }]
        );
        Ok(())
    }

    #[test]
    fn unknown_category_is_rejected() {
        let parsed = serde_json::from_str::<SlopFlag>(
            r#"{"id":"abc","spans":[{"field":"response","text":"x","category":"vibes"}]}"#,
        );
        assert!(parsed.is_err());
    }

    #[test]
    fn missing_flags_file_means_no_flags() -> Result<(), DataError> {
        let flags_by_id = read(Path::new("labels/does-not-exist.jsonl"))?;
        assert!(flags_by_id.is_empty());
        Ok(())
    }
}
