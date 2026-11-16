//! The Edge-LLM drafter build note, with the recorded Thor builds: Gemma 4 26B-A4B with its
//! draft model built at an input of 65,536 (48 GiB of workspace), and Gemma 4 31B with its
//! draft model failed at 131,072 (96 GiB). Both have a vocabulary of 262,144.

use thor_tigress_cli::edge::{
    BuildOutlook, GIBIBYTE, RecordedBuilds, drafter_build_note, gibibytes, largest_input_len,
    lm_head_bytes, max_input_len,
};

/// The vocabulary of both Gemma 4 checkpoints, from their `config.json`.
const GEMMA_VOCAB: u64 = 262_144;

/// The defaults: 48 GiB built, 96 GiB failed.
const RECORDED: RecordedBuilds = RecordedBuilds {
    built: 48 * GIBIBYTE,
    failed: 96 * GIBIBYTE,
};

/// The note's outlook for a context, with the prompt `thor` writes (context minus 8,192).
fn outlook_at(context: u64) -> Option<BuildOutlook> {
    drafter_build_note(Some(GEMMA_VOCAB), context, context - 8192, RECORDED)
        .map(|note| note.outlook)
}

#[test]
fn the_workspace_follows_the_issue_arithmetic() {
    assert_eq!(lm_head_bytes(131_072, GEMMA_VOCAB), 65_536 * 262_144 * 6);
    assert_eq!(gibibytes(lm_head_bytes(131_072, GEMMA_VOCAB)), "96.0 GiB");
    assert_eq!(gibibytes(lm_head_bytes(65_536, GEMMA_VOCAB)), "48.0 GiB");
}

#[test]
fn the_input_is_capped_at_half_the_context() {
    assert_eq!(max_input_len(262_144, 253_952), 131_072);
    assert_eq!(max_input_len(262_144, 32_768), 32_768);
}

#[test]
fn gemma_31b_at_an_input_of_131072_is_likely_to_fail() {
    let note = drafter_build_note(Some(GEMMA_VOCAB), 262_144, 253_952, RECORDED);
    let text = note
        .as_ref()
        .map(|note| note.text.as_str())
        .unwrap_or_default();

    assert_eq!(
        note.as_ref().map(|note| note.outlook),
        Some(BuildOutlook::LikelyToFail)
    );
    assert!(text.contains("(max input 131,072 / 2) × vocabulary 262,144 × 6 bytes = 96.0 GiB"));
    assert!(
        text.contains("The longest input known to build with this vocabulary is 65,536 tokens")
    );
    assert!(text.contains("serve the model with vLLM"));
}

#[test]
fn an_input_of_65536_matches_the_26b_build_and_gets_no_note() {
    assert_eq!(outlook_at(131_072), None);
    assert_eq!(largest_input_len(GEMMA_VOCAB, RECORDED.built), 65_536);
}

#[test]
fn an_input_between_the_recorded_builds_is_untested() {
    let note = drafter_build_note(Some(GEMMA_VOCAB), 196_608, 188_416, RECORDED);
    let text = note
        .as_ref()
        .map(|note| note.text.as_str())
        .unwrap_or_default();

    assert_eq!(
        note.as_ref().map(|note| note.outlook),
        Some(BuildOutlook::Untested)
    );
    assert!(text.contains("= 72.0 GiB for the LM head. This size is untested on this Thor"));
    assert!(text.contains("builds needing 96.0 GiB failed here, and 48.0 GiB built."));
}

#[test]
fn short_inputs_and_unknown_vocabularies_get_no_note() {
    assert_eq!(outlook_at(65_536), None);
    assert_eq!(drafter_build_note(None, 262_144, 253_952, RECORDED), None);
}
