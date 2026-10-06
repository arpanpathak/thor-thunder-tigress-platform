//! Writing responses to a client: whole responses, the CORS preflight answer,
//! and server-sent event streams.

use std::io::Write;

use crate::error::AgentError;

/// Sent with every response, so pages and tools on other sites may call this
/// server. Safe because access is by a key in a header, not by cookies: a
/// site can't use a visitor's key without having it.
pub const CORS: &str = "Access-Control-Allow-Origin: *\r\n";

/// The statuses this server answers with itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 200: here it is.
    Ok,
    /// 401: missing or wrong access key.
    Unauthorized,
    /// 404: no such page.
    NotFound,
}

impl Status {
    fn line(self) -> &'static str {
        match self {
            Status::Ok => "200 OK",
            Status::Unauthorized => "401 Unauthorized",
            Status::NotFound => "404 Not Found",
        }
    }
}

/// Writes a complete response.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn respond(stream: &mut impl Write, status: Status, content_type: &str, body: &[u8]) -> Result<(), AgentError> {
    write!(
        stream,
        "HTTP/1.1 {}\r\n{CORS}Content-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status.line(),
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(stream.flush()?)
}

/// Refuses a request without the right access key, in the error format
/// OpenAI clients expect.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn unauthorized(stream: &mut impl Write) -> Result<(), AgentError> {
    let body = br#"{"error":{"code":401,"message":"Invalid API Key","type":"authentication_error"}}"#;
    respond(stream, Status::Unauthorized, "application/json", body)
}

/// Answers a CORS preflight: browsers ask before a cross-site request that
/// carries an `Authorization` header.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn preflight(stream: &mut impl Write) -> Result<(), AgentError> {
    write!(
        stream,
        "HTTP/1.1 204 No Content\r\n{CORS}Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, x-api-key, anthropic-version\r\nAccess-Control-Max-Age: 86400\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )?;
    Ok(stream.flush()?)
}

/// Starts an event-stream response; events follow with [`send_event`], and
/// the response ends when the connection closes.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn start_events(stream: &mut impl Write) -> Result<(), AgentError> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\n{CORS}Content-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    )?;
    Ok(stream.flush()?)
}

/// Writes one server-sent event carrying `data`.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn send_event(stream: &mut impl Write, data: &str) -> Result<(), AgentError> {
    write!(stream, "data: {data}\n\n")?;
    Ok(stream.flush()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(write: impl FnOnce(&mut Vec<u8>) -> Result<(), AgentError>) -> Result<String, AgentError> {
        let mut out = Vec::new();
        write(&mut out)?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    #[test]
    fn a_response_carries_status_cors_length_and_body() -> Result<(), AgentError> {
        let text = written(|out| respond(out, Status::Ok, "text/plain", b"hi"))?;
        assert!(text.starts_with("HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\n"));
        assert!(text.contains("Content-Type: text/plain\r\nContent-Length: 2\r\n"));
        assert!(text.ends_with("\r\n\r\nhi"));
        Ok(())
    }

    #[test]
    fn every_status_has_its_line() {
        let lines: Vec<&str> = [Status::Ok, Status::Unauthorized, Status::NotFound]
            .into_iter()
            .map(Status::line)
            .collect();
        assert_eq!(lines, ["200 OK", "401 Unauthorized", "404 Not Found"]);
    }

    #[test]
    fn unauthorized_uses_the_openai_error_shape() -> Result<(), AgentError> {
        let text = written(unauthorized)?;
        assert!(text.starts_with("HTTP/1.1 401 Unauthorized"));
        assert!(text.ends_with(r#"{"error":{"code":401,"message":"Invalid API Key","type":"authentication_error"}}"#));
        Ok(())
    }

    #[test]
    fn preflight_allows_the_key_headers() -> Result<(), AgentError> {
        let text = written(preflight)?;
        assert!(text.starts_with("HTTP/1.1 204 No Content"));
        assert!(text.contains("Access-Control-Allow-Headers: Authorization, Content-Type, x-api-key, anthropic-version"));
        Ok(())
    }

    #[test]
    fn events_follow_the_event_stream_header() -> Result<(), AgentError> {
        let text = written(|out| {
            start_events(out)?;
            send_event(out, "[DONE]")
        })?;
        assert!(text.contains("Content-Type: text/event-stream"));
        assert!(text.ends_with("\r\n\r\ndata: [DONE]\n\n"));
        Ok(())
    }
}
