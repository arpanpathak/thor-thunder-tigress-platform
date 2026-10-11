//! Anthropic's `/v1/messages` and `/v1/messages/count_tokens` (used by Claude
//! Code), passed to an engine with the three rewrites it needs: the model
//! named as the engine knows it, system messages moved to the top, and
//! thinking off unless the request asks for it.

use std::io::Write;

use serde_json::{Value, json};

use crate::{
    body::{self, Fields},
    config::Upstreams,
    error::Outcome,
};

/// The field an engine reads to switch the model's chat template.
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
    fn asked_by(fields: &Fields) -> Thinking {
        let kind = fields
            .get("thinking")
            .and_then(|thinking| thinking.get("type"))
            .and_then(Value::as_str);

        match kind {
            Some("enabled" | "adaptive") => Thinking::On,
            _ => Thinking::Off,
        }
    }
}

/// Sends an Anthropic request to `path` on the engine serving its model and
/// relays the answer.
///
/// # Errors
///
/// `AgentError::Json` or `AgentError::BadRequest` when `body` isn't a JSON
/// object; upstream and I/O errors as in [`crate::upstream::Endpoint::post`].
pub fn forward(client: &mut dyn Write, path: &str, body: &[u8], upstreams: &Upstreams) -> Outcome {
    let mut fields = body::fields(body)?;
    let engine = upstreams.serving(body::model(&fields));

    body::name(&mut fields, &engine.model);
    lift_system_messages(&mut fields);
    choose_thinking(&mut fields);

    engine
        .endpoint
        .post(path, &serde_json::to_vec(&fields)?)?
        .relay(client)
}

/// An engine ignores Anthropic's `thinking` field, so the choice is made
/// through the chat template instead. A request that sets the template
/// settings itself is left as it is.
fn choose_thinking(fields: &mut Fields) {
    match (
        Thinking::asked_by(fields),
        fields.contains_key(TEMPLATE_SETTINGS),
    ) {
        (Thinking::Off, false) => {
            fields.insert(
                TEMPLATE_SETTINGS.to_string(),
                json!({ "enable_thinking": false }),
            );
        }
        (Thinking::On, _) | (Thinking::Off, true) => {}
    }
}

/// Moves `{"role":"system"}` messages into the top-level `system`, behind
/// whatever the request already had there, keeping their blocks in order.
///
/// An engine answers `messages[i].role must be user or assistant`, and Claude
/// Code puts parts of its system prompt into the messages as well as at the
/// top level, which is where Anthropic defines them.
fn lift_system_messages(fields: &mut Fields) {
    let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) else {
        return;
    };

    let (system, kept): (Vec<Value>, Vec<Value>) = std::mem::take(messages)
        .into_iter()
        .partition(|message| message.get(ROLE).and_then(Value::as_str) == Some(SYSTEM));
    *messages = kept;

    let lifted: Vec<Value> = system
        .iter()
        .flat_map(|message| blocks(message.get(CONTENT)))
        .collect();

    if lifted.is_empty() {
        return;
    }

    let merged = blocks(fields.get(SYSTEM))
        .into_iter()
        .chain(lifted)
        .collect();
    fields.insert(SYSTEM.to_string(), Value::Array(merged));
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
    use crate::{
        config::Engine,
        error::AgentError,
        paths,
        testing::{FakeServer, FakeWeb, json_response},
        upstream::Endpoint,
    };

    fn upstreams(engine: &FakeServer) -> Upstreams {
        Upstreams {
            default: Engine::new("qwen".to_string(), engine.address()),
            others: Vec::new(),
            search: Endpoint::new(engine.address(), None),
            web: Box::new(FakeWeb::default()),
        }
    }

    /// What the engine was sent for `request`.
    fn sent(request: &Value) -> Outcome<Value> {
        let engine = FakeServer::start(vec![json_response(r#"{"type":"message"}"#)])?;
        let body = serde_json::to_vec(request)?;

        forward(&mut Vec::new(), paths::MESSAGES, &body, &upstreams(&engine))?;

        let seen = engine.requests()?;
        let json = seen[0].split("\r\n\r\n").nth(1).unwrap_or_default();
        Ok(serde_json::from_str(json)?)
    }

    fn thinking_sent(request: &Value) -> Outcome<Value> {
        Ok(sent(request)?
            .pointer("/chat_template_kwargs/enable_thinking")
            .cloned()
            .unwrap_or(Value::Null))
    }

    #[test]
    fn turns_thinking_off_unless_asked() -> Outcome {
        assert_eq!(thinking_sent(&json!({"messages": []}))?, json!(false));
        assert_eq!(
            thinking_sent(&json!({"thinking": {"type": "disabled"}}))?,
            json!(false)
        );
        assert_eq!(
            thinking_sent(&json!({"thinking": {"type": "enabled"}}))?,
            Value::Null
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
    fn a_body_that_is_not_an_object_is_refused() -> Outcome {
        let engine = FakeServer::start(Vec::new())?;
        let upstreams = upstreams(&engine);
        let answered = |body: &[u8]| forward(&mut Vec::new(), paths::MESSAGES, body, &upstreams);

        assert!(matches!(answered(b"[1,2]"), Err(AgentError::BadRequest(_))));
        assert!(matches!(answered(b"{"), Err(AgentError::Json(_))));
        Ok(())
    }

    /// What Claude Code sends: a hosted model's name, and the system prompt
    /// split between the top-level field and a message in the conversation.
    #[test]
    fn claude_code_is_answered_by_the_engine() -> Outcome {
        let lifted = sent(&json!({
            "model": "claude-sonnet-5-5",
            "system": [{"type": "text", "text": "header"}],
            "messages": [
                {"role": "user", "content": "ask"},
                {"role": "system", "content": [{"type": "text", "text": "rules"}]},
                {"role": "assistant", "content": "answer"}
            ]
        }))?;

        assert_eq!(lifted["model"], "qwen");
        assert_eq!(
            lifted[MESSAGES],
            json!([
                {"role": "user", "content": "ask"},
                {"role": "assistant", "content": "answer"}
            ])
        );
        assert_eq!(
            lifted[SYSTEM],
            json!([{"type": "text", "text": "header"}, {"type": "text", "text": "rules"}])
        );
        Ok(())
    }

    #[test]
    fn a_system_message_joins_a_plain_system_prompt() -> Outcome {
        let lifted = sent(&json!({
            "system": "header",
            "messages": [{"role": "system", "content": "rules"}, {"role": "user", "content": "hi"}]
        }))?;

        assert_eq!(
            lifted[SYSTEM],
            json!([{"type": "text", "text": "header"}, {"type": "text", "text": "rules"}])
        );
        assert_eq!(lifted[MESSAGES].as_array().map(Vec::len), Some(1));
        Ok(())
    }

    #[test]
    fn a_request_without_a_system_message_gains_no_system_prompt() -> Outcome {
        let request = json!({"messages": [{"role": "user", "content": "hi"}, {"role": "system"}]});
        let lifted = sent(&request)?;

        assert_eq!(lifted.get(SYSTEM), None);
        assert_eq!(lifted[MESSAGES], json!([{"role": "user", "content": "hi"}]));
        assert_eq!(sent(&json!({"messages": "text"}))?[MESSAGES], "text");
        Ok(())
    }

    #[test]
    fn relays_the_answer_from_the_path_asked_for() -> Outcome {
        let engine = FakeServer::start(vec![json_response(r#"{"input_tokens":3}"#)])?;
        let mut client = Vec::new();

        forward(&mut client, paths::COUNT_TOKENS, b"{}", &upstreams(&engine))?;

        let seen = engine.requests()?;
        assert!(seen[0].starts_with("POST /v1/messages/count_tokens HTTP/1.1"));
        assert!(String::from_utf8_lossy(&client).ends_with(r#"{"input_tokens":3}"#));
        Ok(())
    }
}
