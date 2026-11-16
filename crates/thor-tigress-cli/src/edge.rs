//! A TensorRT Edge-LLM build limit that a memory estimate alone misses.
//!
//! When a checkpoint has a draft model for multi-token prediction (MTP), the draft graph
//! applies the language-model head to every row of the prefill, and TensorRT's autotuner
//! sizes a workspace for it during the build. The workspace is the rows times the
//! vocabulary times 6 bytes: the 2-byte FP16 output of the LM head and its 4-byte FP32 copy.
//! The rows are half the engine's longest input.
//!
//! The autotuner's capacity is not a fixed number. Two failed Gemma 4 31B builds on the
//! Thor reported 35.3 GiB and 32.6 GiB (NVIDIA/TensorRT-Edge-LLM issue #285), and a
//! Gemma 4 26B-A4B build that needed 48 GiB succeeded. `thor` therefore compares the
//! workspace with two recorded points: the largest workspace known to build here
//! (`EDGE_WORKSPACE_BUILT_GIB`, 48) and the smallest known to fail
//! (`EDGE_WORKSPACE_FAILED_GIB`, 96).

use std::path::{Path, PathBuf};

use crate::memory::tokens;

/// Bytes per entry of the LM head's autotuner workspace: 2 for FP16 output, 4 for its FP32 copy.
pub const LM_HEAD_BYTES_PER_ENTRY: u64 = 6;

/// Bytes in a gibibyte, the unit TensorRT reports.
pub const GIBIBYTE: u64 = 1 << 30;

/// How a build with a draft model compares with the recorded builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildOutlook {
    /// The workspace is no larger than one that built here.
    KnownToBuild,
    /// The workspace lies between the recorded build and the recorded failure.
    Untested,
    /// The workspace is at least as large as one that failed here.
    LikelyToFail,
}

/// A note about a build with a draft model, and how it compares with the recorded builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildNote {
    /// The comparison.
    pub outlook: BuildOutlook,
    /// The text shown in the plan, with the arithmetic.
    pub text: String,
}

/// The workspaces recorded on this machine, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedBuilds {
    /// The largest workspace known to build.
    pub built: u64,
    /// The smallest workspace known to fail.
    pub failed: u64,
}

/// The longest input `thor-tigress-serve` builds the engine for: `EDGE_PROMPT`, never more
/// than half the context.
pub fn max_input_len(context: u64, prompt: u64) -> u64 {
    prompt.min(context / 2)
}

/// The autotuner workspace the LM head of the draft graph needs for an input length.
pub fn lm_head_bytes(max_input_len: u64, vocab_size: u64) -> u64 {
    max_input_len / 2 * vocab_size * LM_HEAD_BYTES_PER_ENTRY
}

/// The longest input whose LM-head workspace is no larger than `workspace` bytes.
pub fn largest_input_len(vocab_size: u64, workspace: u64) -> u64 {
    workspace / (vocab_size * LM_HEAD_BYTES_PER_ENTRY) * 2
}

/// The folder a draft model for `model_folder` would be in: `EDGE_ROOT/drafters/NAME`.
pub fn drafter_folder(edge_root: &Path, model_folder: &str) -> PathBuf {
    edge_root.join("drafters").join(model_folder)
}

/// Where a workspace falls between the recorded build and the recorded failure.
pub fn outlook(workspace: u64, recorded: RecordedBuilds) -> BuildOutlook {
    match (workspace <= recorded.built, workspace >= recorded.failed) {
        (true, _) => BuildOutlook::KnownToBuild,
        (false, true) => BuildOutlook::LikelyToFail,
        (false, false) => BuildOutlook::Untested,
    }
}

/// A note for a build with a draft model whose workspace is larger than any known to
/// build here. `None` when it is no larger, or when the vocabulary size is unknown.
pub fn drafter_build_note(
    vocab_size: Option<u64>,
    context: u64,
    prompt: u64,
    recorded: RecordedBuilds,
) -> Option<BuildNote> {
    let vocab_size = vocab_size?;
    let input = max_input_len(context, prompt);
    let workspace = lm_head_bytes(input, vocab_size);
    let arithmetic = format!(
        "with its draft model, Edge-LLM's build needs (max input {} / 2) × vocabulary {} × {} bytes \
         = {} for the LM head.",
        tokens(input),
        tokens(vocab_size),
        LM_HEAD_BYTES_PER_ENTRY,
        gibibytes(workspace)
    );

    let found = outlook(workspace, recorded);
    let text = match found {
        BuildOutlook::KnownToBuild => return None,
        BuildOutlook::Untested => untested_text(&arithmetic, recorded),
        BuildOutlook::LikelyToFail => likely_to_fail_text(&arithmetic, vocab_size, recorded),
    };

    Some(BuildNote {
        outlook: found,
        text,
    })
}

/// The note for a workspace between the recorded build and the recorded failure.
fn untested_text(arithmetic: &str, recorded: RecordedBuilds) -> String {
    format!(
        "Note: {arithmetic} This size is untested on this Thor; builds needing {} failed here, \
         and {} built.",
        gibibytes(recorded.failed),
        gibibytes(recorded.built)
    )
}

/// The note for a workspace at least as large as the recorded failure, with the longest
/// input known to build and the vLLM suggestion.
fn likely_to_fail_text(arithmetic: &str, vocab_size: u64, recorded: RecordedBuilds) -> String {
    let known_input = largest_input_len(vocab_size, recorded.built);

    format!(
        "Warning: {arithmetic} Builds needing {} or more failed on this Thor after about 30 \
         minutes, so this one is likely to fail. The longest input known to build with this \
         vocabulary is {} tokens (--context {} keeps it). For longer prompts, serve the model \
         with vLLM.",
        gibibytes(recorded.failed),
        tokens(known_input),
        known_input * 2
    )
}

/// A size in gibibytes with one decimal, such as `96.0 GiB`.
pub fn gibibytes(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / GIBIBYTE as f64)
}
