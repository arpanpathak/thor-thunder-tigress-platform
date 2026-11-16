//! `lasso agent`: coding tasks from merged pull requests, agent runs on those
//! tasks, run scores, and training data from passed runs.
//!
//! ```text
//! lasso agent tasks  --repo OWNER/NAME --out tasks.jsonl [--gh PROGRAM]
//! lasso agent run    --tasks tasks.jsonl --split train|eval --openbatrangs PATH
//!                    --out runs.jsonl [--timeout SECONDS] [--check-timeout SECONDS]
//!                    [--clone-url TEMPLATE] [-- EXTRA OPENBATRANGS ARGUMENTS]
//! lasso agent score  runs.jsonl
//! lasso agent export runs.jsonl --out train_agent.jsonl [--labels FILE]
//! ```
//!
//! Teacher runs for training and student runs on the held-out tasks use the
//! same commands; they differ only in the openbatrangs arguments after `--`
//! and the output path.

pub mod arguments;
pub mod export;
pub mod jsonl;
pub mod process;
pub mod run;
pub mod score;
pub mod task;
pub mod tasks;

#[cfg(test)]
mod tests;

use std::io::Write;

use crate::error::DistillError;
use arguments::Arguments;

/// Returns the usage text of `lasso agent`.
#[must_use]
pub fn usage() -> String {
    "usage: lasso agent tasks --repo OWNER/NAME --out FILE [--gh PROGRAM]\n       \
     lasso agent run --tasks FILE --split train|eval --openbatrangs PATH --out FILE \
     [--timeout SECONDS] [--check-timeout SECONDS] [--clone-url TEMPLATE] [-- ARGS]\n       \
     lasso agent score RUNS\n       \
     lasso agent export RUNS --out FILE [--labels FILE]"
        .to_string()
}

/// Executes the `lasso agent` subcommand named by the first of `arguments`,
/// with the rest as its arguments. Progress and results go to `out`.
///
/// # Errors
///
/// [`DistillError::Usage`] for arguments that can't be read, and any error of
/// the subcommand.
pub fn main_with(arguments: &[String], out: &mut impl Write) -> Result<(), DistillError> {
    let (command, rest) = arguments
        .split_first()
        .ok_or_else(|| DistillError::Usage(usage()))?;
    let parsed = Arguments::parse(rest)?;

    match command.as_str() {
        "tasks" => tasks::main_with(&parsed, out),
        "run" => run::main_with(&parsed, out),
        "score" => score::main_with(&parsed, out),
        "export" => export::main_with(&parsed, out),
        _ => Err(DistillError::Usage(usage())),
    }
}
