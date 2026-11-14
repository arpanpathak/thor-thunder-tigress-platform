//! `lasso agent score`: pass rate, step and outcome counts of a runs file.

use std::{io::Write, path::PathBuf};

use super::{arguments::Arguments, jsonl, run::RunRecord};
use crate::error::DistillError;

/// Totals of a runs file.
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    /// Runs in the file.
    pub runs: usize,
    /// Runs whose check passed.
    pub passed: usize,
    /// Mean agent steps over all runs; 0 for no runs.
    pub mean_steps: f64,
    /// Runs with outcome `step_limit`.
    pub step_limit: usize,
    /// Runs with outcome `error`.
    pub errors: usize,
    /// Runs with outcome `timeout`.
    pub timeouts: usize,
}

impl Score {
    /// Returns the totals of `records`.
    #[must_use]
    pub fn of(records: &[RunRecord]) -> Self {
        let with = |outcome: &str| {
            records
                .iter()
                .filter(|record| record.outcome == outcome)
                .count()
        };
        let steps: u64 = records.iter().map(|record| record.steps).sum();

        Score {
            runs: records.len(),
            passed: records.iter().filter(|record| record.passed).count(),
            mean_steps: ratio(steps as f64, records.len()),
            step_limit: with("step_limit"),
            errors: with("error"),
            timeouts: with("timeout"),
        }
    }

    /// Returns the totals as a plain two-column table, one row per line.
    #[must_use]
    pub fn table(&self) -> String {
        let rate = 100.0 * ratio(self.passed as f64, self.runs);
        let rows = [
            ("runs", self.runs.to_string()),
            ("passed", format!("{} ({rate:.1}%)", self.passed)),
            ("mean steps", format!("{:.1}", self.mean_steps)),
            ("step limit", self.step_limit.to_string()),
            ("errors", self.errors.to_string()),
            ("timeouts", self.timeouts.to_string()),
        ];

        rows.iter()
            .map(|(name, value)| format!("{name:<12}{value}\n"))
            .collect()
    }
}

/// Returns `total / count`, or 0 when `count` is 0.
fn ratio(total: f64, count: usize) -> f64 {
    if count == 0 {
        return 0.0;
    }

    total / count as f64
}

/// Executes `lasso agent score RUNS` and writes the table to `out`.
///
/// # Errors
///
/// [`DistillError::Usage`] without exactly one runs file, and any error of
/// reading it.
pub fn main_with(arguments: &Arguments, out: &mut impl Write) -> Result<(), DistillError> {
    arguments.allow_only(&[])?;

    let path = PathBuf::from(arguments.single("runs file")?);
    let records: Vec<RunRecord> = jsonl::read(&path)?;

    write!(out, "{}", Score::of(&records).table()).map_err(DistillError::io("standard output"))
}
