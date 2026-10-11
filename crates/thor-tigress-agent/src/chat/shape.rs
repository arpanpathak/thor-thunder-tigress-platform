//! The shape TensorRT Edge-LLM takes a chat request in. Coding tools send
//! what a hosted API accepts; the engine differs in three ways, and all three
//! are settled here before the request leaves.

use serde_json::{Map, Value, json};

use crate::body::Fields;

/// The request field holding the conversation.
const MESSAGES: &str = "messages";

/// Every top-level field TensorRT Edge-LLM 0.11.0 accepts, from the
/// `ChatCompletionRequest` schema in its `/openapi.json`. It answers any
/// other field with 400 `Extra inputs are not permitted`, and clients send
/// plenty: `user`, `store`, `metadata`, `service_tier`, llama.cpp's samplers.
const ENGINE_FIELDS: [&str; 34] = [
    "add_generation_prompt",
    "apply_chat_template",
    "audio",
    "cache_generated_tokens",
    "chat_template_kwargs",
    "disable_spec_decode",
    "enable_thinking",
    "frequency_penalty",
    "guided_decoding",
    "logit_bias",
    "logprobs",
    "max_completion_tokens",
    "max_tokens",
    "messages",
    "min_p",
    "modalities",
    "model",
    "n",
    "parallel_tool_calls",
    "presence_penalty",
    "reasoning_effort",
    "repetition_penalty",
    "response_format",
    "reuse_context",
    "seed",
    "stop",
    "stream",
    "stream_options",
    "temperature",
    "tool_choice",
    "tools",
    "top_k",
    "top_logprobs",
    "top_p",
];

/// Makes `fields` a request the engine takes: only the fields it knows, one
/// system message, first, and earlier tool calls with their arguments as
/// objects.
pub fn fit(fields: &mut Fields) {
    fields.retain(|field, _| ENGINE_FIELDS.contains(&field.as_str()));
    gather_system(fields);
    unpack_call_arguments(fields);
}

/// Turns the arguments of every earlier tool call from JSON text into the
/// object it encodes.
///
/// OpenAI's API carries a call's arguments as a string, and that is what a
/// coding tool sends back in the history. The engine's chat templates walk the
/// arguments as a mapping, so a string renders as a call with no arguments at
/// all: the model reads its own last step as `write_file()` with nothing in
/// it, concludes the step went wrong, and does it again. Measured on both
/// models: the same history is 471 prompt tokens with string arguments and 518
/// with objects.
fn unpack_call_arguments(fields: &mut Fields) {
    let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) else {
        return;
    };
    messages
        .iter_mut()
        .filter_map(|message| message.get_mut("tool_calls")?.as_array_mut())
        .flatten()
        .filter_map(|call| call.pointer_mut("/function/arguments"))
        .for_each(unpack);
}

/// Replaces JSON text that encodes an object with that object; anything else
/// is left as it is.
fn unpack(arguments: &mut Value) {
    let object = arguments
        .as_str()
        .and_then(|text| serde_json::from_str::<Map<String, Value>>(text).ok());

    if let Some(object) = object {
        *arguments = Value::Object(object);
    }
}

/// Joins every system and developer message into one system message at the
/// top. Qwen's chat template stops with `System message must be at the
/// beginning` on a second one or a late one, which the engine reports as a
/// 500, and coding tools send both.
fn gather_system(fields: &mut Fields) {
    let Some(messages) = fields.get_mut(MESSAGES).and_then(Value::as_array_mut) else {
        return;
    };
    let (system, mut kept): (Vec<Value>, Vec<Value>) =
        std::mem::take(messages).into_iter().partition(is_system);
    let text = system
        .iter()
        .map(text_of)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    if !system.is_empty() {
        kept.insert(0, json!({ "role": "system", "content": text }));
    }
    *messages = kept;
}

/// Whether a message is instructions rather than conversation. `developer` is
/// what newer OpenAI clients call the system role.
fn is_system(message: &Value) -> bool {
    matches!(
        message.get("role").and_then(Value::as_str),
        Some("system" | "developer")
    )
}

/// A message's text: its content when that is a string, or the text parts
/// joined when it is a list of parts.
fn text_of(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{body, error::Outcome};

    fn fitted(request: &Value) -> Outcome<Fields> {
        let mut fields = body::fields(&serde_json::to_vec(request)?)?;
        fit(&mut fields);
        Ok(fields)
    }

    #[test]
    fn keeps_every_field_the_engine_knows_and_no_other() -> Outcome {
        let mut request = Fields::new();
        for field in ENGINE_FIELDS {
            request.insert(field.to_string(), json!(1));
        }
        for field in ["user", "store", "metadata", "n_predict", "grammar"] {
            request.insert(field.to_string(), json!(1));
        }
        let kept = fitted(&Value::Object(request))?;
        let mut names: Vec<&str> = kept.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ENGINE_FIELDS);
        Ok(())
    }

    #[test]
    fn system_messages_become_one_at_the_top() -> Outcome {
        let kept = fitted(&json!({ "messages": [
            { "role": "system", "content": "rules" },
            { "role": "user", "content": "ask" },
            { "role": "developer", "content": [
                { "type": "text", "text": "more" },
                { "type": "image_url", "image_url": {} },
                { "type": "text", "text": "rules" }
            ] },
            { "role": "system" },
            { "role": "assistant", "content": "answer" }
        ] }))?;
        assert_eq!(
            kept[MESSAGES],
            json!([
                { "role": "system", "content": "rules\n\nmore\n\nrules" },
                { "role": "user", "content": "ask" },
                { "role": "assistant", "content": "answer" }
            ])
        );
        Ok(())
    }

    #[test]
    fn earlier_tool_calls_carry_their_arguments_as_objects() -> Outcome {
        let call = |arguments: Value| json!({ "id": "c", "function": { "name": "f", "arguments": arguments } });
        let kept = fitted(&json!({ "messages": [
            { "role": "user", "content": "ask" },
            { "role": "assistant", "tool_calls": [
                call(json!("{\"path\": \"a.rs\"}")),
                call(json!({ "path": "b.rs" })),
                call(json!("not json")),
                call(json!("[1]")),
                { "id": "bare" }
            ] },
            { "role": "tool", "tool_call_id": "c", "content": "done" }
        ] }))?;
        let arguments: Vec<&Value> = kept[MESSAGES][1]["tool_calls"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|call| call.pointer("/function/arguments"))
            .collect();
        assert_eq!(
            arguments,
            [
                &json!({ "path": "a.rs" }),
                &json!({ "path": "b.rs" }),
                &json!("not json"),
                &json!("[1]")
            ]
        );
        Ok(())
    }

    #[test]
    fn a_conversation_without_a_system_message_is_left_as_it_is() -> Outcome {
        let request = json!({ "messages": [{ "role": "user", "content": "ask" }] });
        assert_eq!(fitted(&request)?[MESSAGES], request[MESSAGES]);
        assert_eq!(fitted(&json!({ "messages": "text" }))?[MESSAGES], "text");
        Ok(())
    }
}
