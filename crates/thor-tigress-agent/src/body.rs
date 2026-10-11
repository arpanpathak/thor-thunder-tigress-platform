//! A JSON request body as its top-level fields, and the model it names.

use serde_json::{Map, Value};

use crate::error::{AgentError, Outcome};

/// A request's top-level fields.
pub type Fields = Map<String, Value>;

/// The request field naming the model, which picks the engine.
const MODEL: &str = "model";

/// Reads `body` as a JSON object.
///
/// # Errors
///
/// `AgentError::Json` when `body` isn't JSON, and `AgentError::BadRequest`
/// when it is JSON but not an object.
pub fn fields(body: &[u8]) -> Outcome<Fields> {
    match serde_json::from_slice(body)? {
        Value::Object(fields) => Ok(fields),
        _ => Err(AgentError::bad_request("the body must be a JSON object")),
    }
}

/// The model the request names, when it names one.
#[must_use]
pub fn model(fields: &Fields) -> Option<&str> {
    fields.get(MODEL).and_then(Value::as_str)
}

/// Makes the request name `model`, the one its engine serves. An engine
/// answers any other name with 404, and a coding tool sends the name it was
/// built around.
pub fn name(fields: &mut Fields, model: &str) {
    fields.insert(MODEL.to_string(), Value::String(model.to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_object_and_refuses_anything_else() -> Outcome {
        assert_eq!(model(&fields(br#"{"model":"qwen"}"#)?), Some("qwen"));
        assert_eq!(model(&fields(br#"{"model":7}"#)?), None);
        assert_eq!(model(&fields(b"{}")?), None);
        assert!(matches!(fields(b"[1,2]"), Err(AgentError::BadRequest(_))));
        assert!(matches!(fields(b"{"), Err(AgentError::Json(_))));
        Ok(())
    }

    #[test]
    fn names_the_model_over_whatever_was_there() -> Outcome {
        let mut request = fields(br#"{"model":"claude-sonnet-5-5"}"#)?;

        name(&mut request, "qwen");

        assert_eq!(model(&request), Some("qwen"));
        Ok(())
    }
}
