//! Anthropic's `/v1/messages` (used by Claude Code), passed to the model server
//! with the little rewriting the two servers need: thinking off unless the
//! request asks for it, and, for an engine, system messages moved to the top.

use std::io::Write;

use serde_json::{Value, json};

use crate::{error::Outcome, paths, upstream::Endpoint};

/// The field llama-server reads to switch the model's chat template.
const TEMPLATE_SETTINGS: &str = "chat_template_kwargs";

/// The request's conversation.
const MESSAGES: &str = "messages";

/// The role a message carries.
const ROLE: &str = "role";

/// A message's content.
const CONTENT: &str = "content";

/// Anthropic's system prompt: a top-level field, and the only place an engine
/// reads one from.
const SYSTEM: &str = "system";

/// Whether a request wants the model to think before answering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Thinking {
    /// `"thinking": {"type": "enabled"}` or `"adaptive"`.
    On,
    /// Anything else, including no `thinking` field.
    Off,
}

impl Thinking {
    fn asked_by(request: &Value) -> Thinking {
        match request.pointer("/thinking/type").and_then(Value::as_str) {
            Some("enabled" | "adaptive") => Thinking::On,
            _ => Thinking::Off,
        }
    }
}

/// Sends a `/v1/messages` request to the model server and relays the answer.
/// `engine` says whether an engine rather than llama-server will answer it,
/// which decides how much of the body has to be rewritten.
///
/// # Errors
///
/// `AgentError::Json` when `body` isn't JSON; upstream and I/O errors as in
/// [`Endpoint::post`].
pub fn forward(client: &mut dyn Write, body: &[u8], model: &Endpoint, engine: bool) -> Outcome {
    model
        .post(paths::MESSAGES, &prepare(body, engine)?)?
        .relay(client)
}

/// Two rewrites, because the two servers read the same body differently.
///
/// llama-server ignores Anthropic's `thinking` field, so the choice is made
/// through the chat template instead. A request that sets the template settings
/// itself is left as it is.
///
/// An engine takes Anthropic's request as it is except for one field: it answers
/// `messages[i].role must be user or assistant`, and Claude Code puts parts of
/// its system prompt into the messages as well as at the top level. For an
/// engine those move up into `system`, which is where Anthropic defines them and
/// where the engine reads them.
fn prepare(body: &[u8], engine: bool) -> Outcome<Vec<u8>> {
    let mut request: Value = serde_json::from_slice(body)?;
    let thinking = Thinking::asked_by(&request);

    if engine {
        lift_system_messages(&mut request);
    }
    if let Some(fields) = request.as_object_mut()
        && thinking == Thinking::Off
        && !fields.contains_key(TEMPLATE_SETTINGS)
    {
        fields.insert(
            TEMPLATE_SETTINGS.to_string(),
            json!({ "enable_thinking": false }),
        );
    }
    Ok(serde_json::to_vec(&request)?)
}

/// Moves `{"role":"system"}` messages into the top-level `system`, behind
/// whatever the request already had there, keeping their blocks in order.
fn lift_system_messages(request: &mut Value) {
    let Some(fields) = request.as_object_mut() else {
        return;
    };
    let lifted = {
        let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) else {
            return;
        };
        let mut lifted = Vec::new();
        messages.retain(|message| {
            if message.get(ROLE).and_then(Value::as_str) != Some(SYSTEM) {
                return true;
            }
            lifted.extend(blocks(message.get(CONTENT)));
            false
        });
        lifted
    };
    if lifted.is_empty() {
        return;
    }
    let system = fields.entry(SYSTEM.to_string()).or_insert_with(|| json!([]));
    let mut merged = blocks(Some(&*system));
    merged.extend(lifted);
    *system = Value::Array(merged);
}

/// A message's content as blocks: a plain string becomes one text block.
fn blocks(content: Option<&Value>) -> Vec<Value> {
    match content {
        Some(Value::String(text)) => vec![json!({ "type": "text", "text": text })],
        Some(Value::Array(blocks)) => blocks.clone(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, json_response};

    fn prepared(body: &Value, engine: bool) -> Outcome<Value> {
        Ok(serde_json::from_slice(&prepare(
            &serde_json::to_vec(body)?,
            engine,
        )?)?)
    }

    fn thinking_sent(body: &Value) -> Outcome<Value> {
        Ok(prepared(body, false)?
            .pointer("/chat_template_kwargs/enable_thinking")
            .cloned()
            .unwrap_or(Value::Null))
    }

    #[test]
    fn reads_what_the_request_asks_for() {
        assert_eq!(
            Thinking::asked_by(&json!({"thinking": {"type": "enabled"}})),
            Thinking::On
        );
        assert_eq!(
            Thinking::asked_by(&json!({"thinking": {"type": "adaptive"}})),
            Thinking::On
        );
        assert_eq!(
            Thinking::asked_by(&json!({"thinking": {"type": "disabled"}})),
            Thinking::Off
        );
        assert_eq!(Thinking::asked_by(&json!({})), Thinking::Off);
    }

    #[test]
    fn turns_thinking_off_unless_asked() -> Outcome {
        assert_eq!(thinking_sent(&json!({"messages": []}))?, json!(false));
        assert_eq!(
            thinking_sent(&json!({"thinking": {"type": "disabled"}}))?,
            json!(false)
        );
        assert_eq!(
            thinking_sent(&json!({"thinking": {"type": "adaptive"}}))?,
            Value::Null
        );
        assert_eq!(
            thinking_sent(&json!({"chat_template_kwargs": {"enable_thinking": true}}))?,
            json!(true)
        );
        Ok(())
    }

    #[test]
    fn a_body_that_is_not_an_object_passes_unchanged() -> Outcome {
        assert_eq!(prepare(b"[1,2]", false)?, b"[1,2]");
        assert_eq!(prepare(b"[1,2]", true)?, b"[1,2]");
        assert!(prepare(b"{", false).is_err());
        Ok(())
    }

    /// What Claude Code sends: the system prompt split between the top-level
    /// field and a message in the middle of the conversation.
    #[test]
    fn an_engine_is_not_given_a_system_message() -> Outcome {
        let request = json!({
            "system": [{"type": "text", "text": "header"}],
            "messages": [
                {"role": "user", "content": "ask"},
                {"role": "system", "content": [{"type": "text", "text": "rules"}]},
                {"role": "assistant", "content": "answer"}
            ]
        });

        let lifted = prepared(&request, true)?;
        let roles = lifted[MESSAGES]
            .as_array()
            .map(|messages| messages.iter().filter_map(|one| one[ROLE].as_str()).collect::<Vec<_>>());
        assert_eq!(roles, Some(vec!["user", "assistant"]));
        let system = lifted[SYSTEM].as_array().cloned().unwrap_or_default();
        assert_eq!(system.len(), 2);
        assert_eq!(system[0]["text"], "header");
        assert_eq!(system[1]["text"], "rules");

        let kept = prepared(&request, false)?;
        assert_eq!(
            kept[MESSAGES].as_array().map(Vec::len),
            Some(3),
            "llama-server reads its system messages where they are"
        );
        assert_eq!(kept[SYSTEM].as_array().map(Vec::len), Some(1));
        Ok(())
    }

    #[test]
    fn a_system_message_joins_a_plain_system_prompt() -> Outcome {
        let lifted = prepared(
            &json!({
                "system": "header",
                "messages": [{"role": "system", "content": "rules"}, {"role": "user", "content": "hi"}]
            }),
            true,
        )?;
        let system = lifted[SYSTEM].as_array().cloned().unwrap_or_default();
        assert_eq!(system.len(), 2);
        assert_eq!(system[0]["text"], "header");
        assert_eq!(system[1]["text"], "rules");
        assert_eq!(
            lifted[MESSAGES].as_array().map(Vec::len),
            Some(1),
            "the message itself is gone"
        );
        Ok(())
    }

    #[test]
    fn an_engine_request_without_a_system_message_is_left_alone() -> Outcome {
        let request = json!({"messages": [{"role": "user", "content": "hi"}]});
        let lifted = prepared(&request, true)?;
        assert_eq!(lifted.get(SYSTEM), None, "no empty system prompt is added");
        assert_eq!(lifted[MESSAGES], request[MESSAGES]);
        Ok(())
    }

    #[test]
    fn forwards_and_relays() -> Outcome {
        let server = FakeServer::start(vec![json_response(r#"{"type":"message"}"#)])?;
        let mut client = Vec::new();
        forward(
            &mut client,
            br#"{"messages":[]}"#,
            &Endpoint::new(server.address(), None),
            false,
        )?;
        let sent = server.requests()?;
        assert!(sent[0].starts_with("POST /v1/messages HTTP/1.1"));
        assert!(sent[0].contains(r#""enable_thinking":false"#));
        assert!(String::from_utf8_lossy(&client).ends_with(r#"{"type":"message"}"#));
        Ok(())
    }

    #[test]
    fn forwards_an_engine_request_with_the_system_lifted() -> Outcome {
        let server = FakeServer::start(vec![json_response(r#"{"type":"message"}"#)])?;
        let mut client = Vec::new();
        forward(
            &mut client,
            br#"{"messages":[{"role":"user","content":"a"},{"role":"system","content":"b"}]}"#,
            &Endpoint::new(server.address(), None),
            true,
        )?;
        let sent = server.requests()?;
        assert!(!sent[0].contains(r#""role":"system""#), "{}", sent[0]);
        assert!(sent[0].contains(r#""text":"b""#), "{}", sent[0]);
        Ok(())
    }
}
