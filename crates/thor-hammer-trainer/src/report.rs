//! The markdown summary written next to the training file, as `stats.md`.
//!
//! It lists examples and tokens per source, the examples too long for an 8k
//! context, the number of preference pairs, every reason something was left
//! out, and the reviewer's slop spans per category.

use crate::{
    example::{Example, SkipReason, Source},
    slop_flags::{SlopCategory, UnmatchedFlag},
};

/// A rough average for English and code; good enough to size a training run.
pub const CHARS_PER_TOKEN: usize = 4;

/// Examples longer than this do not fit an 8k training context and get cut.
const LONG_EXAMPLE_TOKENS: usize = 8_192;

/// The numbers the report shows, gathered by `main.rs`.
pub struct ReportInput<'a> {
    pub training_set: &'a [Example],
    pub skip_reasons: &'a [SkipReason],
    pub preference_pair_count: usize,
    pub slop_span_counts: &'a [(SlopCategory, usize)],
    pub unmatched_flags: &'a [UnmatchedFlag],
}

/// The whole report as markdown.
pub fn render(input: &ReportInput) -> String {
    let mut report_lines = vec!["# Training set".to_string(), String::new()];
    report_lines.extend(source_table(input.training_set));
    report_lines.push(String::new());
    report_lines.push(format!(
        "Examples over {LONG_EXAMPLE_TOKENS} tokens: {}",
        long_example_count(input.training_set)
    ));
    report_lines.push(format!(
        "Preference pairs in preferences.jsonl: {}",
        input.preference_pair_count
    ));
    report_lines.push(String::new());
    report_lines.extend(left_out_list(input.skip_reasons));
    report_lines.push(String::new());
    report_lines.extend(slop_span_list(input.slop_span_counts));
    report_lines.push(String::new());
    report_lines.extend(unmatched_flag_list(input.unmatched_flags));
    report_lines.push(String::new());
    report_lines.join("\n")
}

/// The flags that matched no example, so they were not applied.
///
/// They are listed rather than counted: a lost flag has to be re-pointed, and
/// that needs its note. An empty list means every flag found its example.
fn unmatched_flag_list(unmatched_flags: &[UnmatchedFlag]) -> Vec<String> {
    let mut list_lines = vec![
        "## Flags that matched no example".to_string(),
        String::new(),
        "These flags are in `labels/slop_flags.jsonl`, but no example in this build".to_string(),
        "has their id, so they were NOT applied. Re-point or clear them in the review tool."
            .to_string(),
        String::new(),
    ];

    if unmatched_flags.is_empty() {
        list_lines.push("- none".to_string());
    }

    for unmatched in unmatched_flags {
        list_lines.push(format!("- `{}`: {}", unmatched.id, unmatched.note));
    }
    list_lines
}

/// Examples and estimated tokens per source, with a total row.
fn source_table(training_set: &[Example]) -> Vec<String> {
    let mut table_lines = vec![
        "| Source | Examples | Tokens (estimate) |".to_string(),
        "|---|---|---|".to_string(),
    ];

    for source in Source::ALL {
        let from_source: Vec<&Example> = training_set
            .iter()
            .filter(|example| example.source == source)
            .collect();
        let source_tokens: usize = from_source
            .iter()
            .map(|example| estimated_tokens(example))
            .sum();
        table_lines.push(format!(
            "| {} | {} | {source_tokens} |",
            source.name(),
            from_source.len()
        ));
    }
    let total_tokens: usize = training_set.iter().map(estimated_tokens).sum();
    table_lines.push(format!(
        "| total | {} | {total_tokens} |",
        training_set.len()
    ));
    table_lines
}

/// One line per [`SkipReason`] with its count.
fn left_out_list(skip_reasons: &[SkipReason]) -> Vec<String> {
    let mut list_lines = vec!["## Left out".to_string(), String::new()];

    for reason in SkipReason::ALL {
        let reason_count = skip_reasons
            .iter()
            .filter(|skipped| **skipped == reason)
            .count();
        list_lines.push(format!("- {reason_count} {}", reason.description()));
    }
    list_lines
}

/// One line per slop category with the number of spans a reviewer marked.
fn slop_span_list(slop_span_counts: &[(SlopCategory, usize)]) -> Vec<String> {
    let mut list_lines = vec![
        "## Slop spans marked by a reviewer".to_string(),
        String::new(),
    ];

    for (category, span_count) in slop_span_counts {
        list_lines.push(format!("- {span_count} {}", category.description()));
    }
    list_lines
}

/// The number of examples that do not fit an 8k context.
fn long_example_count(training_set: &[Example]) -> usize {
    training_set
        .iter()
        .filter(|example| estimated_tokens(example) > LONG_EXAMPLE_TOKENS)
        .count()
}

/// The approximate token count of one example.
fn estimated_tokens(example: &Example) -> usize {
    example.char_count() / CHARS_PER_TOKEN
}
