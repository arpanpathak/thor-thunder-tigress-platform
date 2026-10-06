//! What a chat request gets when it does not say otherwise: the author's
//! rules for code as a system prompt, and a low temperature.
//!
//! Measured on 2026-10-06 with five coding prompts on Nemotron 3 Nano: with
//! these defaults, `else` branches in the code fell from 7 to 1, comments in
//! function bodies from 9 to 3, answers passing all five rules rose from 0
//! to 2 of 5, and answers came back three times faster. A request that sets
//! its own system prompt or temperature keeps them.

use serde_json::{Map, Value, json};

/// The system prompt added to a conversation that has none.
pub const SYSTEM_PROMPT: &str = "You write Rust that a reader understands on the first pass.
Rules for every Rust answer:
- Never call unwrap() or expect(). Use ?, let-else, if let, let chains, or return an Option/Result.
- Errors are a hand-written enum implementing Display and std::error::Error. No anyhow, no thiserror, no String errors.
- Every pub item has a /// doc comment.
- No comments inside function bodies. Explain in doc comments or in prose after the code.
- No index loops like for i in 0..n; use iterators.
- Prefer guard clauses and early returns to nested if/else. Prefer an exhaustive match on enums.
- Prefer patterns (let &x = ..., let Reverse(x) = ...) to * dereferences.
When the user pastes code, change only what they ask for and keep the rest as it is.
Never claim the code follows a rule unless it does, and never say you ran or tested it.
Answer with the code and at most three short sentences. No emoji, no headings that restate the request, no closing summary.";

/// The sampling temperature when the request sets none. Nemotron's own
/// default, 1.0, is meant for open conversation; code that has to follow
/// exact rules comes out more consistent lower down.
pub const TEMPERATURE: f64 = 0.3;

/// The request field holding the conversation.
const MESSAGES: &str = "messages";

/// The request field holding the temperature.
const TEMPERATURE_FIELD: &str = "temperature";

/// Adds the system prompt when no message has the `system` role, and the
/// temperature when none is set. Everything the request does set is kept.
pub fn apply(fields: &mut Map<String, Value>) {
    if let Some(Value::Array(messages)) = fields.get_mut(MESSAGES)
        && !messages.iter().any(|message| message.get("role").and_then(Value::as_str) == Some("system"))
    {
        messages.insert(0, json!({ "role": "system", "content": SYSTEM_PROMPT }));
    }
    fields.entry(TEMPERATURE_FIELD).or_insert_with(|| json!(TEMPERATURE));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    #[test]
    fn adds_the_rules_and_the_temperature_to_a_bare_request() {
        let mut request = fields(json!({ "messages": [{ "role": "user", "content": "median" }] }));
        apply(&mut request);
        assert_eq!(request["messages"][0]["role"], "system");
        assert_eq!(request["messages"][0]["content"], SYSTEM_PROMPT);
        assert_eq!(request["messages"][1]["content"], "median");
        assert_eq!(request["temperature"], json!(TEMPERATURE));
    }

    #[test]
    fn keeps_what_the_request_sets() {
        let mut request = fields(json!({
            "messages": [{ "role": "system", "content": "mine" }, { "role": "user", "content": "q" }],
            "temperature": 0.9
        }));
        apply(&mut request);
        assert_eq!(request["messages"].as_array().map(Vec::len), Some(2));
        assert_eq!(request["messages"][0]["content"], "mine");
        assert_eq!(request["temperature"], json!(0.9));
    }

    #[test]
    fn a_request_without_messages_only_gets_the_temperature() {
        let mut request = fields(json!({ "prompt": "x" }));
        apply(&mut request);
        assert_eq!(request.get("messages"), None);
        assert_eq!(request["temperature"], json!(TEMPERATURE));
    }
}
