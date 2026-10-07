//! Adds answer scores up into the rates a run is judged on.

use std::{cmp::Ordering, collections::BTreeMap};

use serde::Serialize;

use crate::{
    answer::AnswerScore,
    rules::{Rule, Verdict},
    slop::{Category, EM_DASH_HABIT},
};

/// How one rule fared over the answers it applied to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RuleCount {
    /// Answers that passed it.
    pub passed: usize,
    /// Answers it applied to.
    pub applied: usize,
}

/// What a cell shows when there is nothing to divide by.
const NO_VALUE: &str = "–";

/// Counts over a set of answers.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    /// Answers scored.
    pub answers: usize,
    /// Answers with at least one Rust block.
    pub with_code: usize,
    /// Rust blocks seen.
    pub blocks: usize,
    /// Rust blocks that parsed.
    pub parsed_blocks: usize,
    /// Answers with code where every block parsed and no rule failed.
    pub all_rules: usize,
    /// Per rule: answers that passed it and answers it applied to.
    pub rules: BTreeMap<Rule, RuleCount>,
    /// Answers with no slop phrase and no em-dash habit.
    pub clean_prose: usize,
    /// Slop phrases found over all answers.
    pub slop_hits: usize,
    /// Answers with two or more em dashes.
    pub em_dash_habit: usize,
    /// Words of prose over all answers.
    pub words: usize,
    /// Answers with at least one false claim of following the rules.
    pub false_claims: usize,
    /// Slop phrases found, per category.
    pub categories: BTreeMap<Category, usize>,
}

impl Summary {
    /// Adds one answer.
    pub fn add(&mut self, score: &AnswerScore) {
        self.answers += 1;
        self.with_code += usize::from(!score.blocks.is_empty());
        self.blocks += score.blocks.len();
        self.parsed_blocks += score
            .blocks
            .iter()
            .filter(|block| block.report.parsed)
            .count();
        self.all_rules += usize::from(score.all_rules == Some(true));
        for (rule, verdict) in &score.rules {
            let count = self.rules.entry(*rule).or_default();
            count.passed += usize::from(*verdict == Verdict::Pass);
            count.applied += usize::from(*verdict != Verdict::NotApplicable);
        }
        self.clean_prose += usize::from(score.slop.clean());
        self.slop_hits += score.slop.hits.len();
        self.em_dash_habit += usize::from(score.slop.em_dashes >= EM_DASH_HABIT);
        self.words += score.slop.words;
        self.false_claims += usize::from(!score.false_claims.is_empty());
        for hit in &score.slop.hits {
            *self.categories.entry(hit.category).or_default() += 1;
        }
    }

    /// The header and divider of the markdown table [`Summary::row`] fills.
    #[must_use]
    pub fn header() -> String {
        let rules: Vec<&str> = Rule::ALL.iter().map(|rule| rule.label()).collect();
        let columns = [
            vec!["Run", "n", "With code", "Parses", "All 5 rules"],
            rules,
            vec![
                "False claim",
                "Clean prose",
                "Slop / 1k words",
                "Em-dash habit",
                "Words / answer",
            ],
        ]
        .concat();
        let divider = vec!["---"; columns.len()];
        format!("| {} |\n|{}|", columns.join(" | "), divider.join("|"))
    }

    /// One markdown table row. Rule rates are over the answers the rule
    /// applied to; the count is shown next to the rate.
    #[must_use]
    pub fn row(&self, label: &str) -> String {
        let rules = Rule::ALL.iter().map(|rule| {
            let count = self.rules.get(rule).copied().unwrap_or_default();
            format!(
                "{} of {}",
                percent(count.passed, count.applied),
                count.applied
            )
        });
        let cells: Vec<String> = [
            label.to_string(),
            self.answers.to_string(),
            self.with_code.to_string(),
            percent(self.parsed_blocks, self.blocks),
            percent(self.all_rules, self.with_code),
        ]
        .into_iter()
        .chain(rules)
        .chain([
            percent(self.false_claims, self.answers),
            percent(self.clean_prose, self.answers),
            per_thousand(self.slop_hits, self.words),
            percent(self.em_dash_habit, self.answers),
            match self.answers {
                0 => NO_VALUE.to_string(),
                answers => (self.words / answers).to_string(),
            },
        ])
        .collect();
        format!("| {} |", cells.join(" | "))
    }
}

/// `part` of `whole` as a whole-number percentage, or a dash for none.
#[must_use]
pub fn percent(part: usize, whole: usize) -> String {
    match whole {
        0 => NO_VALUE.to_string(),
        whole => format!("{}%", rounded(part * 100, whole)),
    }
}

/// `count` per thousand `words`, with one decimal, or a dash for none.
fn per_thousand(count: usize, words: usize) -> String {
    match words {
        0 => NO_VALUE.to_string(),
        words => {
            let tenths = rounded(count * 10_000, words);
            format!("{}.{}", tenths / 10, tenths % 10)
        }
    }
}

/// `numerator / denominator` to the nearest whole number, a half rounding to
/// the even neighbour, the way `{:.0}` formats a float. `denominator` is not zero.
fn rounded(numerator: usize, denominator: usize) -> usize {
    let quotient = numerator / denominator;
    match (numerator % denominator * 2).cmp(&denominator) {
        Ordering::Less => quotient,
        Ordering::Greater => quotient + 1,
        Ordering::Equal => quotient + quotient % 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer::score;

    #[test]
    fn counts_rules_only_where_they_apply() {
        let mut summary = Summary::default();
        summary.add(&score("Plain words."));
        summary.add(&score("```rust\nfn a() { b().unwrap(); }\n```"));
        assert_eq!(summary.with_code, 1);
        assert_eq!(
            summary.rules.get(&Rule::NoUnwrap),
            Some(&RuleCount {
                passed: 0,
                applied: 1
            })
        );
        assert_eq!(
            summary.rules.get(&Rule::ErrorEnum),
            Some(&RuleCount {
                passed: 0,
                applied: 0
            })
        );
        assert_eq!(summary.all_rules, 0);
    }

    #[test]
    fn a_row_has_one_cell_per_header_column() {
        let header_cells = Summary::header()
            .lines()
            .next()
            .map_or(0, |line| line.matches('|').count());
        let row_cells = Summary::default().row("empty").matches('|').count();
        assert_eq!(header_cells, row_cells);
    }

    #[test]
    fn percent_of_nothing_is_a_dash() {
        assert_eq!(percent(0, 0), "–");
        assert_eq!(percent(1, 4), "25%");
    }

    #[test]
    fn rounds_halves_to_even_like_float_formatting() {
        let integer: Vec<String> = [(1, 8), (3, 8), (5, 8), (1, 200), (2, 3), (1, 3)]
            .iter()
            .map(|&(part, whole)| percent(part, whole))
            .collect();
        assert_eq!(integer, ["12%", "38%", "62%", "0%", "67%", "33%"]);
        assert_eq!(
            [
                per_thousand(1, 16),
                per_thousand(1, 3),
                per_thousand(0, 9),
                per_thousand(1, 0)
            ],
            ["62.5", "333.3", "0.0", "–"]
        );
    }

    #[test]
    fn a_row_reports_rates_slop_and_words() {
        let mut summary = Summary::default();
        summary.add(&score(
            "Great question! Use iter().\n```rust\n/// Adds.\npub fn a() {}\n```",
        ));
        let row = summary.row("run");
        assert!(row.starts_with("| run | 1 | 1 | 100% | 100% |"), "{row}");
        assert!(row.contains("100% of 1"));
    }
}
