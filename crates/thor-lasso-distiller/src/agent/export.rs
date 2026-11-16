//! `lasso agent export`: training lines from passed runs.
//!
//! Each line is `{"tools": [...], "messages": [...]}`, the chat format that
//! `tokenizer.apply_chat_template(messages, tools=tools)` accepts. A run is
//! exported when its check passed and its outcome is `answered`. In assistant
//! messages that carry tool calls, `content` is set to the empty string,
//! because that text is narration of the next step; the tool calls and the
//! final answer are unchanged.
//!
//! `--labels` takes the reinforcer's flags file for runs
//! (`labels/agent_run_flags.jsonl`), one `{"id": RUN_ID, "note": ...,
//! "spans": [...]}` per line. A run whose `run_id` appears there is marked as
//! slop and is not exported.

use std::{collections::HashSet, io::Write, path::PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{arguments::Arguments, jsonl, run::RunRecord};
use crate::error::DistillError;

/// The outcome of a run whose agent gave a final answer.
const ANSWERED: &str = "answered";

/// The run id of a reinforcer flag.
#[derive(Deserialize)]
struct Flag {
    id: String,
}

/// Executes `lasso agent export RUNS --out FILE [--labels FILE]`: writes the
/// training lines to `--out`, replacing its contents, and one summary line to
/// `out`.
///
/// # Errors
///
/// [`DistillError::Usage`] for options that can't be read, and any error of
/// reading the runs or labels or writing the lines.
pub fn main_with(arguments: &Arguments, out: &mut impl Write) -> Result<(), DistillError> {
    arguments.allow_only(&["out", "labels"])?;

    let runs = PathBuf::from(arguments.single("runs file")?);
    let path = PathBuf::from(arguments.required("out")?);
    let records: Vec<RunRecord> = jsonl::read(&runs)?;
    let slop = flagged(arguments.options.get("labels"))?;
    let passed: Vec<&RunRecord> = records.iter().filter(|record| is_kept(record)).collect();
    let lines: Vec<Value> = passed
        .iter()
        .filter(|record| !slop.contains(&record.run_id))
        .map(|record| training_line(&record.transcript))
        .collect();

    jsonl::write(&path, &lines)?;
    writeln!(
        out,
        "{} of {} runs passed, {} marked as slop, {} lines written to {}",
        passed.len(),
        records.len(),
        passed.len() - lines.len(),
        lines.len(),
        path.display()
    )
    .map_err(DistillError::io("standard output"))
}

/// Returns the run ids flagged in the labels file at `path`, or an empty set
/// when `path` is `None`.
///
/// # Errors
///
/// [`DistillError::Io`] for a missing or unreadable file and
/// [`DistillError::Json`] for a line without a string `id`.
fn flagged(path: Option<&String>) -> Result<HashSet<String>, DistillError> {
    let flags: Vec<Flag> = match path {
        Some(path) => jsonl::read(&PathBuf::from(path))?,
        None => Vec::new(),
    };

    Ok(flags.into_iter().map(|flag| flag.id).collect())
}

/// Returns true when the run's check passed and its outcome is `answered`.
#[must_use]
pub fn is_kept(record: &RunRecord) -> bool {
    record.passed && record.outcome == ANSWERED
}

/// Returns `{"tools", "messages"}` from `transcript`, with each message passed
/// through [`without_narration`]. A missing field becomes an empty array.
#[must_use]
pub fn training_line(transcript: &Value) -> Value {
    let messages: Vec<Value> = transcript
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| messages.iter().map(without_narration).collect())
        .unwrap_or_default();
    let tools = transcript.get("tools").cloned().unwrap_or(json!([]));

    json!({ "tools": tools, "messages": messages })
}

/// Returns `message` with `content` set to `""` when it has a non-empty
/// `tool_calls` array, and unchanged otherwise.
#[must_use]
pub fn without_narration(message: &Value) -> Value {
    let mut kept = message.clone();
    let calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|calls| !calls.is_empty());

    if let Some(content) = kept.get_mut("content").filter(|_| calls) {
        *content = json!("");
    }

    kept
}
