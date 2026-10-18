//! Bounds generated text and detects degenerate output before forwarding it.

use serde_json::Value;

/// Consecutive whitespace bytes tolerated in either generated text channel.
const MAX_BLANK: usize = 1024;

/// The bounded suffix used to find exact repeated spans across SSE chunks.
const TAIL_BYTES: usize = 4096;

/// Independent text channels; a transition from reasoning to an answer must
/// not itself count as repetition.
#[derive(Default)]
struct Text {
    tail: Vec<u8>,
    blank: usize,
}

impl Text {
    fn observe(&mut self, text: &str) -> Option<&'static str> {
        for character in text.chars() {
            self.blank = if character.is_whitespace() {
                self.blank.saturating_add(character.len_utf8())
            } else {
                0
            };

            if self.blank >= MAX_BLANK {
                return Some("Stopped because the model kept generating blank output.");
            }
        }

        self.tail.extend_from_slice(text.as_bytes());

        if self.tail.len() > TAIL_BYTES {
            self.tail.drain(..self.tail.len() - TAIL_BYTES);
        }

        let repeated = (32..=512.min(self.tail.len() / 6)).any(|width| {
            let suffix = &self.tail[self.tail.len() - width * 6..];
            let mut spans = suffix.chunks_exact(width);
            let first = spans.next().unwrap_or_default();

            spans.all(|span| span == first)
        });

        repeated.then_some("Stopped because the model kept repeating the same text.")
    }
}

/// One round's output budget and degeneration checks. Tool arguments and
/// metadata are left alone; coding clients with their own tools use the relay.
#[derive(Default)]
pub(super) struct Guard {
    bytes: usize,
    empty: usize,
    content: Text,
    reasoning: Text,
}

impl Guard {
    /// Checks a chunk before any of its text is sent to the browser.
    pub(super) fn observe(&mut self, value: &Value) -> Option<&'static str> {
        self.bytes = self.bytes.saturating_add(super::text_length(value));

        if self.bytes > super::MAX_ROUND_CHARS {
            return Some("Stopped at the response size limit.");
        }

        let delta = &value["choices"][0]["delta"];
        let text = ["content", "reasoning_content", "reasoning"]
            .into_iter()
            .filter_map(|key| delta[key].as_str())
            .collect::<Vec<_>>();

        if !text.is_empty() {
            self.empty = if text.iter().all(|text| text.is_empty()) {
                self.empty.saturating_add(1)
            } else {
                0
            };
        }

        if self.empty >= 128 {
            return Some("Stopped because the model kept generating empty output.");
        }

        if let Some(text) = delta["content"].as_str()
            && let Some(reason) = self.content.observe(text)
        {
            return Some(reason);
        }

        delta["reasoning_content"]
            .as_str()
            .or_else(|| delta["reasoning"].as_str())
            .and_then(|text| self.reasoning.observe(text))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn detects_repetition_split_across_chunks() {
        let mut guard = Guard::default();
        let text = ".or_else(|| ::cuda_oxide::memory::DeviceBuffer::new(len))\n".repeat(10);
        let stopped = text.as_bytes().chunks(7).any(|piece| {
            guard
                .observe(
                    &json!({"choices":[{"delta":{"content": String::from_utf8_lossy(piece)}}]}),
                )
                .is_some()
        });

        assert!(stopped);
    }

    #[test]
    fn blank_reasoning_is_bounded_and_normal_formatting_is_allowed() {
        let mut guard = Guard::default();
        let normal =
            json!({"choices":[{"delta":{"content":"\n\n```rust\n    fn main() {}\n```"}}]});
        let blanks = json!({"choices":[{"delta":{"reasoning":" ".repeat(MAX_BLANK)}}]});

        assert!(guard.observe(&normal).is_none());
        assert!(guard.observe(&blanks).is_some());
    }

    #[test]
    fn channels_are_separate_and_metadata_does_not_spend_the_budget() {
        let mut guard = Guard::default();
        let text = "A sufficiently long sentence repeated only three times. ".repeat(3);

        assert!(
            guard
                .observe(&json!({"choices":[{"delta":{"reasoning_content":text}}]}))
                .is_none()
        );
        assert!(
            guard
                .observe(&json!({"choices":[{"delta":{"content":text}}]}))
                .is_none()
        );
        assert!(
            guard
                .observe(&json!({"usage":{"completion_tokens":9000}}))
                .is_none()
        );
    }
}
