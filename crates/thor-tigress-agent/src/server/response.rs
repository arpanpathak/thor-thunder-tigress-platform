//! What a blocking handler answers on, and the two things it writes there:
//! whole responses and server-sent event streams.
//!
//! The HTTP itself is axum's. A handler is given a [`Client`], tells it the
//! status and content type once with [`Client::begin`], and then writes the
//! body; [`crate::server::bridge`] turns that into the response.

use std::io::Write;

use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::error::{AgentError, Outcome};

/// The last event of every stream.
pub const DONE: &str = "[DONE]";

/// How each line of a server-sent event stream starts.
pub const EVENT_PREFIX: &str = "data:";

/// The content type of the API's answers.
pub const JSON: &str = "application/json";

/// The content type of a stream of events.
pub const EVENT_STREAM: &str = "text/event-stream";

/// A client waiting for an answer. The body is whatever is written to it
/// after [`Client::begin`].
pub trait Client: Write {
    /// Starts the answer with its status and content type.
    ///
    /// # Errors
    ///
    /// `AgentError::Io` when the client is gone or the answer has already begun.
    fn begin(&mut self, status: StatusCode, content_type: &str) -> Outcome;
}

/// Writes a complete response.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn respond(
    client: &mut dyn Client,
    status: StatusCode,
    content_type: &str,
    body: &[u8],
) -> Outcome {
    client.begin(status, content_type)?;
    client.write_all(body)?;

    Ok(client.flush()?)
}

/// The status that explains `error` to a client, when it can be explained: a
/// malformed request gets 400, a failing model or search server 502. `None`
/// for a failure of the connection or the server's own settings, which are
/// not the client's to read.
#[must_use]
pub fn status_of(error: &AgentError) -> Option<StatusCode> {
    match error {
        AgentError::BadRequest(_) | AgentError::Json(_) => Some(StatusCode::BAD_REQUEST),
        AgentError::Keyring(_) => Some(StatusCode::INTERNAL_SERVER_ERROR),
        AgentError::Upstream(_) | AgentError::Fetch(_) | AgentError::Refused(_) => {
            Some(StatusCode::BAD_GATEWAY)
        }
        AgentError::Io(_) | AgentError::Config(_) => None,
    }
}

/// What a client is told about `error`.
fn message(error: &AgentError) -> String {
    json!({ "error": { "message": error.to_string() } }).to_string()
}

/// Tells the client why its request failed, when that can be explained; see
/// [`status_of`]. Otherwise nothing is written.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone or the answer has already begun.
pub fn failure(client: &mut dyn Client, error: &AgentError) -> Outcome {
    match status_of(error) {
        Some(status) => respond(client, status, JSON, message(error).as_bytes()),
        None => Ok(()),
    }
}

impl IntoResponse for AgentError {
    fn into_response(self) -> Response {
        eprintln!("request failed: {self}");

        match status_of(&self) {
            Some(status) => {
                (status, [(header::CONTENT_TYPE, JSON)], message(&self)).into_response()
            }
            None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

/// Starts an event-stream response; events follow with [`send_event`], and
/// the response ends when the handler returns.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn start_events(client: &mut dyn Client) -> Outcome {
    client.begin(StatusCode::OK, EVENT_STREAM)
}

/// Writes one server-sent event carrying `data`, in one piece.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn send_event(stream: &mut dyn Write, data: &str) -> Outcome {
    stream.write_all(format!("{EVENT_PREFIX} {data}\n\n").as_bytes())?;

    Ok(stream.flush()?)
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    fn written(write: impl FnOnce(&mut dyn Client) -> Outcome) -> Outcome<String> {
        let mut out = Vec::new();
        write(&mut out)?;

        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    #[test]
    fn every_writer_reports_a_gone_client() {
        let mut gone = crate::testing::Gone;

        assert!(respond(&mut gone, StatusCode::OK, JSON, b"{}").is_err());
        assert!(start_events(&mut gone).is_err());
        assert!(failure(&mut gone, &AgentError::bad_request("x")).is_err());
        assert!(send_event(&mut gone, DONE).is_err());
        assert!(gone.flush().is_err());
    }

    #[test]
    fn a_response_carries_status_type_and_body() -> Outcome {
        let text = written(|out| respond(out, StatusCode::OK, "text/plain", b"hi"))?;

        assert_eq!(
            text,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhi"
        );

        Ok(())
    }

    #[test]
    fn failures_get_the_status_that_explains_them() -> Outcome {
        let said = |error: AgentError| written(|out| failure(out, &error));
        let bad = said(AgentError::bad_request("the body must be a JSON object"))?;
        let gateway = said(AgentError::Upstream("127.0.0.1:8079: refused".to_string()))?;
        let refused = said(AgentError::Refused("127.0.0.1: not public".to_string()))?;
        let unfetched = said(AgentError::Fetch("page".to_string()))?;
        let sealed = said(AgentError::Keyring("wrong passphrase".to_string()))?;
        let gone = said(AgentError::from(std::io::Error::other("reset")))?;
        let unset = said(AgentError::Config("no engine".to_string()))?;

        assert!(bad.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(
            bad.ends_with(r#"{"error":{"message":"bad request: the body must be a JSON object"}}"#)
        );
        assert!(gateway.starts_with("HTTP/1.1 502 Bad Gateway"));
        assert!(refused.starts_with("HTTP/1.1 502 Bad Gateway"));
        assert!(unfetched.starts_with("HTTP/1.1 502 Bad Gateway"));
        assert!(sealed.starts_with("HTTP/1.1 500 Internal Server Error"));
        assert_eq!((gone.as_str(), unset.as_str()), ("", ""));

        Ok(())
    }

    #[tokio::test]
    async fn an_error_answers_with_its_status_and_message() -> Outcome {
        let explained = AgentError::bad_request("no body").into_response();
        let unexplained = AgentError::from(std::io::Error::other("disk")).into_response();
        let statuses = (explained.status(), unexplained.status());
        let said = to_bytes(explained.into_body(), usize::MAX).await;
        let unsaid = to_bytes(unexplained.into_body(), usize::MAX).await;

        assert_eq!(
            statuses,
            (StatusCode::BAD_REQUEST, StatusCode::INTERNAL_SERVER_ERROR)
        );
        assert!(
            said.is_ok_and(
                |body| body.as_ref() == br#"{"error":{"message":"bad request: no body"}}"#
            )
        );
        assert!(unsaid.is_ok_and(|body| body.is_empty()));

        Ok(())
    }

    #[test]
    fn events_follow_the_event_stream_header() -> Outcome {
        let text = written(|out| {
            start_events(out)?;

            send_event(out, DONE)
        })?;

        assert!(text.contains("Content-Type: text/event-stream"));
        assert!(text.ends_with("\r\n\r\ndata: [DONE]\n\n"));

        Ok(())
    }
}
