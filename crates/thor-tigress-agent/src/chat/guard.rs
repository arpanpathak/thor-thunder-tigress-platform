//! Bounds generated text and detects degenerate output before forwarding it.

use serde_json::Value;

/// Consecutive whitespace bytes tolerated in either generated text channel.
const MAX_BLANK: usize = 1024;

/// A short unit, such as a line or a sentence, counts as a loop after six
/// copies in a row: a table rule or a block of closing braces repeats a few.
const SHORT: Repeat = Repeat {
    widths: 32..=512,
    copies: 6,
};

/// A long unit, such as a paragraph or a whole plan, counts after three
/// copies. Three identical paragraphs in a row are never meant; Qwen3.6 wrote
/// one block of about 900 bytes of thinking over and over.
const LONG: Repeat = Repeat {
    widths: 513..=2048,
    copies: 3,
};

/// The bounded suffix used to find exact repeated spans across SSE chunks:
/// room for three copies of the longest [`LONG`] unit.
const TAIL_BYTES: usize = 6144;

/// Units of a range of widths, repeated a number of times at the end of the
/// text.
struct Repeat {
    /// The unit widths checked, in bytes.
    widths: std::ops::RangeInclusive<usize>,
    /// How many copies in a row make a loop.
    copies: usize,
}

impl Repeat {
    /// Whether `tail` ends in `copies` copies of one unit of these widths.
    fn ends(&self, tail: &[u8]) -> bool {
        self.widths
            .clone()
            .take_while(|width| width * self.copies <= tail.len())
            .any(|width| self.ends_at_width(tail, width))
    }

    /// Whether the last `copies` spans of `width` bytes in `tail` are the
    /// same. The caller makes sure `tail` holds that many.
    fn ends_at_width(&self, tail: &[u8], width: usize) -> bool {
        let mut spans = tail.rchunks_exact(width).take(self.copies);
        let last = spans.next();

        spans.all(|span| Some(span) == last)
    }
}

/// Independent text channels; a transition from reasoning to an answer must
/// not itself count as repetition.
#[derive(Default)]
struct Text {
    /// The last [`TAIL_BYTES`] of the channel's text.
    tail: Vec<u8>,
    /// Whitespace bytes in a row at the end of the channel's text.
    blank: usize,
}

impl Text {
    /// Adds a chunk's text to the channel, and names why the reply must stop
    /// when it runs blank or repeats itself.
    fn observe(&mut self, text: &str) -> Option<&'static str> {
        self.blank_run(text).or_else(|| self.repeated(text))
    }

    /// Counts the whitespace in a row, across chunks, and says when it
    /// reaches [`MAX_BLANK`].
    fn blank_run(&mut self, text: &str) -> Option<&'static str> {
        let too_long = text
            .chars()
            .any(|character| self.count_blank(character) >= MAX_BLANK);

        too_long.then_some("Stopped because the model kept generating blank output.")
    }

    /// Adds `character` to the run of whitespace, or ends the run, and
    /// returns its length in bytes.
    fn count_blank(&mut self, character: char) -> usize {
        self.blank = if character.is_whitespace() {
            self.blank.saturating_add(character.len_utf8())
        } else {
            0
        };

        self.blank
    }

    /// Adds `text` to the tail and says when the tail ends in a short or a
    /// long unit repeated.
    fn repeated(&mut self, text: &str) -> Option<&'static str> {
        self.tail.extend_from_slice(text.as_bytes());
        self.tail
            .drain(..self.tail.len().saturating_sub(TAIL_BYTES));

        let looped = [SHORT, LONG].iter().any(|repeat| repeat.ends(&self.tail));

        looped.then_some("Stopped because the model kept repeating the same text.")
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
    fn a_900_byte_unit_repeated_three_times_in_thinking_is_a_loop() {
        let mut guard = Guard::default();
        let unit = (1..=90)
            .map(|n| format!("step {n:03}; "))
            .collect::<String>();
        let thinking = |text: &str| json!({"choices":[{"delta":{"reasoning_content": text}}]});

        assert_eq!(unit.len(), 900);
        assert!(guard.observe(&thinking(&unit.repeat(2))).is_none());
        assert!(guard.observe(&thinking(&unit)).is_some());
    }

    #[test]
    fn a_long_unit_repeated_twice_is_not_a_loop() {
        let mut guard = Guard::default();
        let unit = (1..=90)
            .map(|n| format!("line {n:03}; "))
            .collect::<String>();
        let answer = json!({"choices":[{"delta":{"content": unit.repeat(2)}}]});

        assert!(guard.observe(&answer).is_none());
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
