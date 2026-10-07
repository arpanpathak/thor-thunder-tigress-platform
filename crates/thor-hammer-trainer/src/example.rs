//! One training example, and the reasons an example can be left out.
//!
//! Every input module produces [`Example`]s. Anything it drops is recorded as a
//! [`SkipReason`], so the report can say how much was left out and why.

use serde::{Deserialize, Serialize};

/// Which input an example came from. Written to `train.jsonl` so training can
/// weigh sources differently, for example repeat the readability set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The hand-written readability_training.md set.
    Readability,
    /// A question and answer from the Claude chat export.
    Chat,
    /// A section of a book or doc chapter.
    Book,
    /// A source file whose header comment describes it.
    Code,
    /// A verified rewrite of clever code into readable code, from the clever-vs-readable set.
    CleverVsReadable,
    /// A section of a book or doc chapter from the fetched open corpus.
    Corpus,
}

impl Source {
    /// Every source, in the order the report lists them.
    pub const ALL: [Source; 6] = [
        Source::Readability,
        Source::CleverVsReadable,
        Source::Corpus,
        Source::Chat,
        Source::Book,
        Source::Code,
    ];

    /// The name used in the report.
    pub fn name(self) -> &'static str {
        match self {
            Source::Readability => "readability",
            Source::Chat => "chat",
            Source::Book => "book",
            Source::Code => "code",
            Source::CleverVsReadable => "clever_vs_readable",
            Source::Corpus => "corpus",
        }
    }
}

/// One request and the answer the model should learn to give, or, when the
/// instruction is empty, a passage of text to learn the writing from. Serialized
/// as one line of `train.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Example {
    /// What the user asks. Empty for a passage of book or doc text.
    pub instruction: String,
    /// The answer to learn.
    pub response: String,
    /// Which input it came from.
    pub source: Source,
    /// The file, or the conversation and turn, it came from, so a reviewer can find it.
    pub origin: String,
}

impl Example {
    /// Length of instruction and response together, in characters.
    pub fn char_count(&self) -> usize {
        self.instruction.chars().count() + self.response.chars().count()
    }

    /// The text compared to find duplicates. Every run of whitespace counts as one
    /// space, so copies that differ only in line breaks match.
    pub fn dedup_key(&self) -> String {
        let instruction_words: Vec<&str> = self.instruction.split_whitespace().collect();
        let response_words: Vec<&str> = self.response.split_whitespace().collect();
        format!(
            "{}\n{}",
            instruction_words.join(" "),
            response_words.join(" ")
        )
    }

    /// A stable id for the example: [`stable_id`] of [`Example::dedup_key`]. It
    /// stays the same across rebuilds as long as the text does, so a reviewer's
    /// slop flag keeps pointing at the right example.
    pub fn id(&self) -> String {
        stable_id(&self.dedup_key())
    }
}

/// The FNV-1a hash of `text`, as 16 hex digits.
pub fn stable_id(text: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;
    let hash = text.bytes().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    });
    format!("{hash:016x}")
}

/// Why an example or a file was left out of the training set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The conversation carries a safety flag.
    SafetyFlag,
    /// The question is about an image the export does not include.
    AboutImage,
    /// The question or the answer has no text.
    EmptyTurn,
    /// The markdown file holds notes about a book, not book content.
    NotesFile,
    /// The source file has no comment to turn into a request.
    NoHeaderComment,
    /// The example is under the minimum length.
    TooShort,
    /// The example repeats an earlier one.
    Duplicate,
    /// A reviewer flagged the example as AI slop on the review page.
    FlaggedSlop,
}

impl SkipReason {
    /// Every reason, in the order the report lists them.
    pub const ALL: [SkipReason; 8] = [
        SkipReason::SafetyFlag,
        SkipReason::AboutImage,
        SkipReason::EmptyTurn,
        SkipReason::NotesFile,
        SkipReason::NoHeaderComment,
        SkipReason::TooShort,
        SkipReason::Duplicate,
        SkipReason::FlaggedSlop,
    ];

    /// What was left out, as a phrase that follows a count in the report.
    pub fn description(self) -> &'static str {
        match self {
            SkipReason::SafetyFlag => "chat turns in conversations with a safety flag",
            SkipReason::AboutImage => "chat questions about an image the export does not include",
            SkipReason::EmptyTurn => "chat turns with an empty question or answer",
            SkipReason::NotesFile => "markdown files of notes, not book content",
            SkipReason::NoHeaderComment => "code files without a header comment",
            SkipReason::TooShort => "examples under the minimum length",
            SkipReason::Duplicate => "duplicate examples",
            SkipReason::FlaggedSlop => {
                "examples flagged as AI slop by a reviewer, written to slop.jsonl"
            }
        }
    }
}
