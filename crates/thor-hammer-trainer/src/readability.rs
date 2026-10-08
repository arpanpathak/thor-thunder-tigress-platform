//! Examples from readability_training.md, the hand-written set.
//!
//! ## Format
//!
//! ```text
//!   ---
//!   ### Instruction
//!   <!-- Tags: iterators -->
//!   What item types does iter() give me?
//!
//!   ### Response
//!   Each one yields a different item type: ...
//!   ---
//! ```
//!
//! Entries are separated by a line holding only `---`. The file's own format
//! notes and every entry's topic tags are HTML comments, so comments are removed
//! before the entries are read.

use crate::example::{Example, Source};

/// Opens an HTML comment.
const COMMENT_START: &str = "<!--";

/// Closes an HTML comment.
const COMMENT_END: &str = "-->";

/// Every instruction and response entry in the file.
pub fn examples(readability_set: &str) -> Vec<Example> {
    let without_comments = remove_html_comments(readability_set);
    let mut examples = Vec::new();

    for entry in without_comments.split("\n---\n") {
        let Some(instruction_and_response) = entry.trim().strip_prefix("### Instruction") else {
            continue;
        };

        let Some((instruction, response)) = instruction_and_response.split_once("### Response")
        else {
            continue;
        };

        examples.push(Example {
            instruction: instruction.trim().to_string(),
            response: response.trim().to_string(),
            source: Source::Readability,
            origin: "readability_training.md".to_string(),
        });
    }
    examples
}

/// The text with every `<!-- ... -->` comment cut out. An unclosed comment runs
/// to the end of the text.
fn remove_html_comments(markdown: &str) -> String {
    let mut kept = String::new();
    let mut remaining = markdown;

    while let Some(comment_start) = remaining.find(COMMENT_START) {
        kept.push_str(&remaining[..comment_start]);
        remaining = match remaining[comment_start..].find(COMMENT_END) {
            Some(comment_length) => {
                &remaining[comment_start + comment_length + COMMENT_END.len()..]
            }
            None => "",
        };
    }
    kept.push_str(remaining);
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_an_entry_without_a_response_and_an_unclosed_comment() {
        let set = "# Set\n---\n### Instruction\nNo answer here.\n---\n### Instruction\nQ\n### Response\nA <!-- never closed";
        let pairs: Vec<(String, String)> = examples(set)
            .into_iter()
            .map(|example| (example.instruction, example.response))
            .collect();
        assert_eq!(pairs, [("Q".to_string(), "A".to_string())]);
    }

    #[test]
    fn reads_entries_and_ignores_comments() {
        let readability_set = "# Title\n<!-- each entry has ### Instruction and ### Response -->\n---\n### Instruction\n<!-- Tags: a -->\nWhy?\n### Response\nBecause.\n---\n### Instruction\nHow?\n### Response\nLike this.\n";
        let pairs: Vec<(String, String)> = examples(readability_set)
            .into_iter()
            .map(|example| (example.instruction, example.response))
            .collect();
        let expected = [
            ("Why?".to_string(), "Because.".to_string()),
            ("How?".to_string(), "Like this.".to_string()),
        ];
        assert_eq!(pairs, expected);
    }
}
