//! Agent-run records: one JSON object per line, each holding the result of
//! one coding task and the transcript from `openbatrangs --transcript-out`.
//!
//! ## Format
//!
//! ```text
//!   {"run_id": "pr-31-a", "task_id": "pr-31", "passed": true, "steps": 7,
//!    "outcome": "answered", "wall_seconds": 84.2,
//!    "transcript": {"model": "...", "tools": [...], "messages": [...],
//!                   "outcome": "answered", "steps": 7}}
//! ```
//!
//! `task_id` and `passed` are required; every other field is optional. The
//! record id is `run_id`, or `task_id` when `run_id` is absent; flags are
//! keyed by this id. The index source is [`PASSED`] or [`FAILED`] and the
//! origin is the task id.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::datasets::index::Header;

/// Source label for runs whose tests passed.
pub const PASSED: &str = "passed";

/// Source label for runs whose tests failed.
pub const FAILED: &str = "failed";

/// Fields of a run line read during indexing; the transcript is skipped.
#[derive(Deserialize)]
struct RunHeader {
    #[serde(default)]
    run_id: Option<String>,
    task_id: String,
    passed: bool,
}

/// Parses the index header of a run line: id, result source and task id.
///
/// # Errors
///
/// Returns the JSON error if `line` is not valid JSON or lacks `task_id` or
/// `passed`.
pub fn header(line: &str) -> serde_json::Result<Header> {
    let run: RunHeader = serde_json::from_str(line)?;

    Ok(Header {
        id: run.run_id.unwrap_or_else(|| run.task_id.clone()),
        source: source_of(run.passed).to_string(),
        origin: run.task_id,
    })
}

/// Returns the source label for a run result.
fn source_of(passed: bool) -> &'static str {
    if passed { PASSED } else { FAILED }
}

/// Run fields sent to the page with each record. All fields are `None` or
/// null for records that are not runs.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RunFields {
    /// Task id.
    #[serde(default)]
    pub task_id: Option<String>,
    /// Whether the task's tests passed after the run.
    #[serde(default)]
    pub passed: Option<bool>,
    /// Number of model calls.
    #[serde(default)]
    pub steps: Option<u64>,
    /// `answered`, `step_limit` or `error`.
    #[serde(default)]
    pub outcome: Option<String>,
    /// Wall-clock duration in seconds.
    #[serde(default)]
    pub wall_seconds: Option<f64>,
    /// Transcript object: `model`, `tools`, `messages` (OpenAI chat format),
    /// `outcome` and `steps`.
    #[serde(default)]
    pub transcript: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_known_by_its_run_id_or_else_its_task() -> serde_json::Result<()> {
        let named = header(r#"{"run_id":"r1","task_id":"pr-9","passed":true}"#)?;
        let unnamed = header(r#"{"task_id":"pr-9","passed":false,"transcript":{}}"#)?;

        assert_eq!(
            (
                named.id.as_str(),
                named.source.as_str(),
                named.origin.as_str()
            ),
            ("r1", PASSED, "pr-9")
        );
        assert_eq!(
            (unnamed.id.as_str(), unnamed.source.as_str()),
            ("pr-9", FAILED)
        );
        assert!(header(r#"{"run_id":"r1","passed":true}"#).is_err());

        Ok(())
    }

    #[test]
    fn a_record_that_is_not_a_run_has_empty_run_fields() -> serde_json::Result<()> {
        let fields: RunFields = serde_json::from_str(r#"{"id":"a","response":"b"}"#)?;

        assert!(fields.task_id.is_none());
        assert!(fields.passed.is_none());
        assert!(fields.transcript.is_null());

        Ok(())
    }
}
