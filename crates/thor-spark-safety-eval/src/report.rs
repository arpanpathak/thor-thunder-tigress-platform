//! Adds answer scores up into the rates a run is judged on.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::{
    answer::AnswerScore,
    rules::{Rule, Verdict},
    slop::{Category, EM_DASH_HABIT},
};

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
    pub rules: BTreeMap<Rule, (usize, usize)>,
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
            let counts = self.rules.entry(*rule).or_default();
            counts.0 += usize::from(*verdict == Verdict::Pass);
            counts.1 += usize::from(*verdict != Verdict::NotApplicable);
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
    pub fn header() -> String {
        let rules: Vec<&str> = Rule::ALL.iter().map(|rule| rule.label()).collect();
        let columns = [
            vec!["Run", "n", "With code", "Parses", "All 5 rules"],
            rules,
            vec!["False claim", "Clean prose", "Slop / 1k words", "Em-dash habit", "Words / answer"],
        ]
        .concat();
        let divider = vec!["---"; columns.len()];
        format!("| {} |\n|{}|", columns.join(" | "), divider.join("|"))
    }

    /// One markdown table row. Rule rates are over the answers the rule
    /// applied to; the count is shown next to the rate.
    pub fn row(&self, label: &str) -> String {
        let rules = Rule::ALL.iter().map(|rule| {
            let (passed, applied) = self.rules.get(rule).copied().unwrap_or_default();
            format!("{} of {applied}", percent(passed, applied))
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
            match self.words {
                0 => "–".to_string(),
                words => format!("{:.1}", self.slop_hits as f64 * 1000.0 / words as f64),
            },
            percent(self.em_dash_habit, self.answers),
            match self.answers {
                0 => "–".to_string(),
                answers => (self.words / answers).to_string(),
            },
        ])
        .collect();
        format!("| {} |", cells.join(" | "))
    }
}

/// `part` of `whole` as a whole-number percentage, or a dash for none.
pub fn percent(part: usize, whole: usize) -> String {
    match whole {
        0 => "–".to_string(),
        whole => format!("{:.0}%", part as f64 * 100.0 / whole as f64),
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
        assert_eq!(summary.rules.get(&Rule::NoUnwrap), Some(&(0, 1)));
        assert_eq!(summary.rules.get(&Rule::ErrorEnum), Some(&(0, 0)));
        assert_eq!(summary.all_rules, 0);
    }

    #[test]
    fn a_row_has_one_cell_per_header_column() {
        let header_cells = Summary::header().lines().next().map_or(0, |line| line.matches('|').count());
        let row_cells = Summary::default().row("empty").matches('|').count();
        assert_eq!(header_cells, row_cells);
    }

    #[test]
    fn percent_of_nothing_is_a_dash() {
        assert_eq!(percent(0, 0), "–");
        assert_eq!(percent(1, 4), "25%");
    }
}
