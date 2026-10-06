//! Anthropic's `/v1/messages` (used by Claude Code), passed to llama-server
//! with one change: thinking off unless the request asks for it.

use std::io::Write;

use serde_json::{Value, json};

use crate::{error::AgentError, upstream::Endpoint};

/// Sends a `/v1/messages` request to the model server and relays the answer.
///
/// # Errors
///
/// `AgentError::Json` when `body` isn't JSON; upstream and I/O errors as in
/// [`Endpoint::post`].
pub fn forward(client: &mut impl Write, body: &[u8], model: &Endpoint) -> Result<(), AgentError> {
    model.post("/v1/messages", &prepare(body)?)?.relay(client)
}

/// llama-server ignores Anthropic's `thinking` field, so the choice is made
/// through the chat template: thinking stays on only for `enabled` or
/// `adaptive`. A request that sets `chat_template_kwargs` itself is left as
/// it is.
fn prepare(body: &[u8]) -> Result<Vec<u8>, AgentError> {
    let mut request: Value = serde_json::from_slice(body)?;
    let asked = matches!(
        request.pointer("/thinking/type").and_then(Value::as_str),
        Some("enabled" | "adaptive")
    );
    if let Some(fields) = request.as_object_mut()
        && !asked
        && !fields.contains_key("chat_template_kwargs")
    {
        fields.insert("chat_template_kwargs".to_string(), json!({ "enable_thinking": false }));
    }
    Ok(serde_json::to_vec(&request)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, json_response};

    fn thinking_sent(body: &Value) -> Result<Value, AgentError> {
        let prepared: Value = serde_json::from_slice(&prepare(&serde_json::to_vec(body)?)?)?;
        Ok(prepared.pointer("/chat_template_kwargs/enable_thinking").cloned().unwrap_or(Value::Null))
    }

    #[test]
    fn thinks_only_when_asked() -> Result<(), AgentError> {
        assert_eq!(thinking_sent(&json!({"messages": []}))?, json!(false));
        assert_eq!(thinking_sent(&json!({"thinking": {"type": "disabled"}}))?, json!(false));
        assert_eq!(thinking_sent(&json!({"thinking": {"type": "adaptive"}}))?, Value::Null);
        assert_eq!(thinking_sent(&json!({"thinking": {"type": "enabled", "budget_tokens": 1024}}))?, Value::Null);
        assert_eq!(thinking_sent(&json!({"chat_template_kwargs": {"enable_thinking": true}}))?, json!(true));
        Ok(())
    }

    #[test]
    fn a_body_that_is_not_an_object_passes_unchanged() -> Result<(), AgentError> {
        assert_eq!(prepare(b"[1,2]")?, b"[1,2]");
        assert!(prepare(b"{").is_err());
        Ok(())
    }

    #[test]
    fn forwards_and_relays() -> Result<(), AgentError> {
        let server = FakeServer::start(vec![json_response(r#"{"type":"message"}"#)])?;
        let mut client = Vec::new();
        forward(&mut client, br#"{"messages":[]}"#, &Endpoint::new(server.address(), None))?;
        let sent = server.requests()?;
        assert!(sent[0].starts_with("POST /v1/messages HTTP/1.1"));
        assert!(sent[0].contains(r#""enable_thinking":false"#));
        assert!(String::from_utf8_lossy(&client).ends_with(r#"{"type":"message"}"#));
        Ok(())
    }
}
