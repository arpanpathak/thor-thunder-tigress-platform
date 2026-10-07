//! Anthropic's `/v1/messages` (used by Claude Code), passed to llama-server
//! with one change: thinking off unless the request asks for it.

use std::io::Write;

use serde_json::{Value, json};

use crate::{error::Outcome, paths, upstream::Endpoint};

/// The field llama-server reads to switch the model's chat template.
const TEMPLATE_SETTINGS: &str = "chat_template_kwargs";

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
///
/// # Errors
///
/// `AgentError::Json` when `body` isn't JSON; upstream and I/O errors as in
/// [`Endpoint::post`].
pub fn forward(client: &mut dyn Write, body: &[u8], model: &Endpoint) -> Outcome {
    model.post(paths::MESSAGES, &prepare(body)?)?.relay(client)
}

/// llama-server ignores Anthropic's `thinking` field, so the choice is made
/// through the chat template instead. A request that sets the template
/// settings itself is left as it is.
fn prepare(body: &[u8]) -> Outcome<Vec<u8>> {
    let mut request: Value = serde_json::from_slice(body)?;
    let thinking = Thinking::asked_by(&request);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, json_response};

    fn thinking_sent(body: &Value) -> Outcome<Value> {
        let prepared: Value = serde_json::from_slice(&prepare(&serde_json::to_vec(body)?)?)?;
        Ok(prepared
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
        assert_eq!(prepare(b"[1,2]")?, b"[1,2]");
        assert!(prepare(b"{").is_err());
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
        )?;
        let sent = server.requests()?;
        assert!(sent[0].starts_with("POST /v1/messages HTTP/1.1"));
        assert!(sent[0].contains(r#""enable_thinking":false"#));
        assert!(String::from_utf8_lossy(&client).ends_with(r#"{"type":"message"}"#));
        Ok(())
    }
}
