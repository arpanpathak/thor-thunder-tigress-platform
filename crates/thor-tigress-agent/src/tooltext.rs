//! Tool calls that arrive as answer text instead of as streamed `tool_calls`.
//!
//! Some engines write the call into the answer in the model's own format, for
//! example Nemotron's:
//!
//! ```text
//! <tool_call>
//! <function=web_search>
//! <parameter=query>
//! rust jobs
//! </parameter>
//! <parameter=time_range>
//! week
//! </parameter>
//! </function>
//! </tool_call>
//! ```
//!
//! and some use JSON inside the tags:
//!
//! ```text
//! <tool_call>{"name": "web_search", "arguments": {"query": "rust jobs"}}</tool_call>
//! ```
//!
//! [`Sieve`] filters a streamed answer: the text around a call is returned as it
//! arrives, the call itself never reaches the reader, and it comes back as a
//! [`TextCall`] the server can run. A tag split across two chunks is held back
//! until it is complete, so neither half is shown.

use serde_json::{Map, Value};

/// The start of a tool-call block; the closing tag shares this text.
const OPEN: &str = "<tool_call";

/// The end of a tool-call block.
const CLOSE: &str = "</tool_call>";

/// The longest a block may stay open before it is let through as text.
const MAX_PENDING: usize = 16 * 1024;

/// One tool call read from the text form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCall {
    /// The tool's name.
    pub name: String,
    /// Its arguments, as a JSON object.
    pub arguments: String,
}

/// Filters a streamed answer, keeping the tool-call blocks out of it.
#[derive(Debug, Default)]
pub struct Sieve {
    pending: String,
    open: bool,
    calls: Vec<TextCall>,
}

impl Sieve {
    /// An empty filter.
    #[must_use]
    pub fn new() -> Self {
        Sieve::default()
    }

    /// Whether a block is open, so the caller knows the answer is being held.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Adds `text` and returns the part of it that is not part of a tool call.
    pub fn push(&mut self, text: &str) -> String {
        self.pending.push_str(text);
        self.drain()
    }

    /// Ends the stream and returns whatever was held back: the tail of an
    /// unfinished tag, or the tail of an unfinished block.
    pub fn finish(&mut self) -> String {
        let pending = std::mem::take(&mut self.pending);
        if self.open {
            self.open = false;
            if let Some(call) = parse(&pending) {
                self.calls.push(call);
                return String::new();
            }
        }
        pending
    }

    /// The calls found so far, leaving the sieve empty of them.
    pub fn take_calls(&mut self) -> Vec<TextCall> {
        std::mem::take(&mut self.calls)
    }

    /// Pulls every complete block out of `pending` and returns the rest.
    fn drain(&mut self) -> String {
        let mut visible = String::new();
        loop {
            if self.open {
                if let Some(end) = find_ci(&self.pending, CLOSE) {
                    let block = self.pending[..end].to_string();
                    self.pending.drain(..end + CLOSE.len());
                    self.open = false;
                    if let Some(call) = parse(&block) {
                        self.calls.push(call);
                    }
                    continue;
                }
                if self.pending.len() > MAX_PENDING {
                    visible.push_str(&std::mem::take(&mut self.pending));
                    self.open = false;
                }
                break;
            }

            let Some(at) = find_ci(&self.pending, OPEN) else {
                let keep = holdback(&self.pending);
                let split = self.pending.len() - keep;
                visible.push_str(&self.pending[..split]);
                self.pending.drain(..split);
                break;
            };
            visible.push_str(&self.pending[..at]);
            self.pending.drain(..at);

            let Some(gt) = self.pending.find('>') else {
                break;
            };
            self.pending.drain(..gt + 1);
            self.open = true;
        }
        visible
    }
}

/// The byte index of `needle` in `haystack`, compared without case.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    let last = haystack.len().checked_sub(needle.len())?;
    (0..=last).find(|start| haystack[*start..*start + needle.len()].eq_ignore_ascii_case(needle))
}

/// How many bytes at the end of `text` could still become [`OPEN`].
fn holdback(text: &str) -> usize {
    let bytes = text.as_bytes();
    let longest = OPEN.len().min(bytes.len());
    (1..=longest)
        .rev()
        .find(|length| {
            bytes[bytes.len() - length..].eq_ignore_ascii_case(&OPEN.as_bytes()[..*length])
        })
        .unwrap_or(0)
}

/// One call from a block, in either the XML or the JSON form.
fn parse(block: &str) -> Option<TextCall> {
    let trimmed = block.trim();
    if trimmed.starts_with('{') {
        parse_json(trimmed)
    } else {
        parse_xml(trimmed)
    }
}

/// `{"name": …, "arguments": …}` inside the tags.
fn parse_json(block: &str) -> Option<TextCall> {
    let value: Value = serde_json::from_str(block).ok()?;
    let name = value
        .get("name")
        .or_else(|| value.get("tool"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }
    let arguments = match value
        .get("arguments")
        .or_else(|| value.get("parameters"))
        .or_else(|| value.get("args"))
    {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "{}".to_string(),
    };
    Some(TextCall { name, arguments })
}

/// `<function=NAME><parameter=KEY>VALUE</parameter>…</function>` inside the tags.
fn parse_xml(block: &str) -> Option<TextCall> {
    let at = find_ci(block, "<function=")?;
    let start = at + "<function=".len();
    let end = block[start..].find('>')? + start;
    let name = block[start..end].trim().to_string();
    if name.is_empty() {
        return None;
    }

    let mut arguments = Map::new();
    let mut rest = &block[end..];
    while let Some(at) = find_ci(rest, "<parameter=") {
        let start = at + "<parameter=".len();
        let Some(gt) = rest[start..].find('>') else {
            break;
        };
        let key = rest[start..start + gt].trim().to_string();
        let value_at = start + gt + 1;
        let value = match find_ci(&rest[value_at..], "</parameter>") {
            Some(end) => {
                let value = rest[value_at..value_at + end].trim().to_string();
                rest = &rest[value_at + end + "</parameter>".len()..];
                value
            }
            None => {
                let value = rest[value_at..].trim().to_string();
                rest = "";
                value
            }
        };
        if !key.is_empty() {
            arguments.insert(key, Value::String(value));
        }
    }
    Some(TextCall {
        name,
        arguments: Value::Object(arguments).to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> (String, Vec<TextCall>) {
        let mut sieve = Sieve::new();
        let mut visible = sieve.push(text);
        visible.push_str(&sieve.finish());
        (visible, sieve.take_calls())
    }

    #[test]
    fn plain_text_passes_through() {
        let (visible, calls) = one("The answer is 42.");
        assert_eq!(visible, "The answer is 42.");
        assert_eq!(calls, []);
    }

    #[test]
    fn an_xml_call_is_parsed_and_never_shown() {
        let block = "<tool_call>\n<function=web_search>\n<parameter=query>\nrust jobs\n</parameter>\n<parameter=time_range>\nweek\n</parameter>\n</function>\n</tool_call>";
        let (visible, calls) = one(&format!("Let me look that up.\n{block}\n"));
        assert_eq!(visible, "Let me look that up.\n\n");
        assert_eq!(
            calls,
            [TextCall {
                name: "web_search".to_string(),
                arguments: r#"{"query":"rust jobs","time_range":"week"}"#.to_string(),
            }]
        );
    }

    #[test]
    fn a_json_call_is_parsed() {
        let (visible, calls) = one(
            r#"<tool_call>{"name": "fetch_page_content_recursive", "arguments": {"url": "https://e/x"}}</tool_call>"#,
        );
        assert_eq!(visible, "");
        assert_eq!(
            calls,
            [TextCall {
                name: "fetch_page_content_recursive".to_string(),
                arguments: r#"{"url":"https://e/x"}"#.to_string(),
            }]
        );
    }

    #[test]
    fn a_call_split_across_chunks_comes_out_whole() {
        let mut sieve = Sieve::new();
        let mut visible = String::new();
        for piece in [
            "Sure. <tool",
            "_call>\n<function=web_",
            "search>\n<parameter=query>\nru",
            "st\n</parameter>\n</function>\n</tool_",
            "call>\nDone.",
        ] {
            visible.push_str(&sieve.push(piece));
        }
        visible.push_str(&sieve.finish());
        assert_eq!(visible, "Sure. \nDone.");
        assert_eq!(sieve.take_calls().len(), 1);
        assert_eq!(sieve.take_calls(), []);
    }

    #[test]
    fn several_calls_in_one_chunk_are_all_parsed() {
        let (visible, calls) = one(
            "<tool_call><function=web_search><parameter=query>a</parameter></function></tool_call>\
             <tool_call><function=web_search><parameter=query>b</parameter></function></tool_call>",
        );
        assert_eq!(visible, "");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].arguments, r#"{"query":"b"}"#);
    }

    #[test]
    fn a_marker_that_never_becomes_a_call_is_swallowed() {
        let (visible, calls) = one("use <tool_call> in your template");
        assert!(!visible.contains("<tool_call"));
        assert_eq!(calls, []);
    }

    #[test]
    fn a_partial_tag_at_the_end_is_flushed_by_finish() {
        let (visible, calls) = one("see the <tool");
        assert_eq!(visible, "see the <tool");
        assert_eq!(calls, []);
    }

    #[test]
    fn a_block_without_a_name_is_not_a_call() {
        let (visible, calls) = one("<tool_call>\n<parameter=query>x</parameter>\n</tool_call>");
        assert_eq!(visible, "");
        assert_eq!(calls, []);
    }

    #[test]
    fn an_unclosed_block_is_flushed_or_parsed_at_the_end() {
        let (visible, calls) =
            one("<tool_call><function=web_search><parameter=query>rust</parameter></function>");
        assert_eq!(visible, "");
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn find_helpers_cover_their_edges() {
        assert_eq!(find_ci("a<Tool_Call>b", OPEN), Some(1));
        assert_eq!(find_ci("nothing", OPEN), None);
        assert_eq!(find_ci("", OPEN), None);
        assert_eq!(holdback("abc"), 0);
        assert_eq!(holdback("<to"), 3);
        assert_eq!(holdback("</to"), 0);
        assert_eq!(parse("not a call"), None);
        assert_eq!(parse(r#"{"arguments": {}}"#), None);
    }

    #[test]
    fn a_json_call_may_carry_a_string_or_no_arguments() {
        let (_, calls) = one(
            r#"<tool_call>{"name":"web_search","arguments":"{\"query\":\"rust\"}"}</tool_call>"#,
        );
        assert_eq!(calls[0].arguments, r#"{"query":"rust"}"#);

        let (_, calls) = one(r#"<tool_call>{"name":"web_search"}</tool_call>"#);
        assert_eq!(calls[0].arguments, "{}");

        let (_, calls) = one(r#"<tool_call>{"name":"  "}</tool_call>"#);
        assert_eq!(calls, []);
    }

    #[test]
    fn an_xml_call_without_a_name_or_a_whole_parameter_is_not_a_call() {
        let (_, calls) = one("<tool_call><function=></function></tool_call>");
        assert_eq!(calls, []);

        let (_, calls) = one("<tool_call><function=x><parameter=q</tool_call>");
        assert_eq!(calls[0].arguments, "{}");

        let (_, calls) = one("<tool_call><function=x><parameter=q>v</tool_call>");
        assert_eq!(calls[0].arguments, r#"{"q":"v"}"#);
    }

    #[test]
    fn an_empty_parameter_name_is_left_out() {
        let (_, calls) =
            one("<tool_call><function=x><parameter=>v</parameter></function></tool_call>");
        assert_eq!(calls[0].arguments, "{}");
    }

    #[test]
    fn an_opening_tag_that_never_finishes_is_flushed() {
        let mut sieve = Sieve::new();
        assert_eq!(sieve.push("x <tool_call"), "x ");
        assert!(!sieve.is_open());
        assert_eq!(sieve.finish(), "<tool_call");
    }

    #[test]
    fn a_block_that_never_closes_lets_its_text_through_once_it_is_too_long() {
        let mut sieve = Sieve::new();
        let huge = format!("<tool_call>{}", "x".repeat(MAX_PENDING + 10));
        let visible = sieve.push(&huge);
        assert!(visible.starts_with("xxx"));
        assert!(!sieve.is_open());
        assert_eq!(sieve.take_calls(), []);
    }
}
