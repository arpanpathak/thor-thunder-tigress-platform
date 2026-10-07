//! Writing responses to a client: whole responses, the CORS preflight answer,
//! and server-sent event streams.

use std::io::Write;

use serde_json::json;

use crate::error::{AgentError, Outcome};

/// Sent with every response, so pages and tools on other sites may call this
/// server. Safe because access is by a key in a header, not by cookies: a
/// site can't use a visitor's key without having it.
pub const CORS: &str = "Access-Control-Allow-Origin: *\r\n";

/// The last event of every stream.
pub const DONE: &str = "[DONE]";

/// How each line of a server-sent event stream starts.
pub const EVENT_PREFIX: &str = "data:";

/// What the server sends back when the access key is missing or wrong, in
/// the error format OpenAI clients expect.
const INVALID_KEY: &[u8] =
    br#"{"error":{"code":401,"message":"Invalid API Key","type":"authentication_error"}}"#;

/// The statuses this server answers with itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 200: here it is.
    Ok,
    /// 401: missing or wrong access key.
    Unauthorized,
    /// 400: the request is malformed.
    BadRequest,
    /// 404: no such page.
    NotFound,
    /// 502: the model server or the search engine failed.
    BadGateway,
}

/// The kinds of body this server sends itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    /// JSON, for the API and `/health`.
    Json,
    /// Plain text, for "not found".
    Text,
    /// The chat page and the About page.
    Html,
    /// The cub as a drawing.
    Svg,
    /// The cub as a picture, for link previews.
    Png,
}

impl Status {
    fn line(self) -> &'static str {
        match self {
            Status::Ok => "200 OK",
            Status::Unauthorized => "401 Unauthorized",
            Status::BadRequest => "400 Bad Request",
            Status::NotFound => "404 Not Found",
            Status::BadGateway => "502 Bad Gateway",
        }
    }
}

impl ContentType {
    fn header(self) -> &'static str {
        match self {
            ContentType::Json => "application/json",
            ContentType::Text => "text/plain",
            ContentType::Html => "text/html; charset=utf-8",
            ContentType::Svg => "image/svg+xml",
            ContentType::Png => "image/png",
        }
    }
}

/// Writes a complete response.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn respond(
    stream: &mut dyn Write,
    status: Status,
    content_type: ContentType,
    body: &[u8],
) -> Outcome {
    write!(
        stream,
        "HTTP/1.1 {}\r\n{CORS}Content-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status.line(),
        content_type.header(),
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(stream.flush()?)
}

/// Refuses a request without the right access key.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn unauthorized(stream: &mut dyn Write) -> Outcome {
    respond(stream, Status::Unauthorized, ContentType::Json, INVALID_KEY)
}

/// Tells the client why its request failed, when there is someone to tell:
/// a malformed request gets 400, a failing model or search server 502. When
/// the connection itself failed, nothing is written.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn failure(stream: &mut dyn Write, error: &AgentError) -> Outcome {
    let status = match error {
        AgentError::BadRequest(_) | AgentError::Json(_) => Status::BadRequest,
        AgentError::Upstream(_) => Status::BadGateway,
        AgentError::Io(_) | AgentError::Config(_) => return Ok(()),
    };
    let body = json!({ "error": { "message": error.to_string() } }).to_string();
    respond(stream, status, ContentType::Json, body.as_bytes())
}

/// Answers a CORS preflight: browsers ask before a cross-site request that
/// carries an `Authorization` header.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn preflight(stream: &mut dyn Write) -> Outcome {
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
pub fn start_events(stream: &mut dyn Write) -> Outcome {
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
pub fn send_event(stream: &mut dyn Write, data: &str) -> Outcome {
    write!(stream, "{EVENT_PREFIX} {data}\n\n")?;
    Ok(stream.flush()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_writer_reports_a_gone_client() {
        let mut gone = crate::testing::Gone;
        assert!(respond(&mut gone, Status::Ok, ContentType::Json, b"{}").is_err());
        assert!(preflight(&mut gone).is_err());
        assert!(start_events(&mut gone).is_err());
        assert!(unauthorized(&mut gone).is_err());
        assert!(failure(&mut gone, &AgentError::bad_request("x")).is_err());
    }

    fn written(write: impl FnOnce(&mut dyn Write) -> Outcome) -> Outcome<String> {
        let mut out = Vec::new();
        write(&mut out)?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    #[test]
    fn a_response_carries_status_cors_length_and_body() -> Outcome {
        let text = written(|out| respond(out, Status::Ok, ContentType::Text, b"hi"))?;
        assert!(text.starts_with("HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\n"));
        assert!(text.contains("Content-Type: text/plain\r\nContent-Length: 2\r\n"));
        assert!(text.ends_with("\r\n\r\nhi"));
        Ok(())
    }

    #[test]
    fn every_status_and_content_type_has_its_header() {
        let lines = [
            Status::Ok,
            Status::Unauthorized,
            Status::BadRequest,
            Status::NotFound,
            Status::BadGateway,
        ]
        .map(Status::line);
        let types = [
            ContentType::Json,
            ContentType::Text,
            ContentType::Html,
            ContentType::Svg,
            ContentType::Png,
        ]
        .map(ContentType::header);
        assert_eq!(
            lines,
            [
                "200 OK",
                "401 Unauthorized",
                "400 Bad Request",
                "404 Not Found",
                "502 Bad Gateway"
            ]
        );
        assert_eq!(
            types,
            [
                "application/json",
                "text/plain",
                "text/html; charset=utf-8",
                "image/svg+xml",
                "image/png"
            ]
        );
    }

    #[test]
    fn unauthorized_uses_the_openai_error_shape() -> Outcome {
        let text = written(unauthorized)?;
        assert!(text.starts_with("HTTP/1.1 401 Unauthorized"));
        assert!(text.ends_with(
            r#"{"error":{"code":401,"message":"Invalid API Key","type":"authentication_error"}}"#
        ));
        Ok(())
    }

    #[test]
    fn failures_get_the_status_that_explains_them() -> Outcome {
        let bad = written(|out| {
            failure(
                out,
                &AgentError::bad_request("the body must be a JSON object"),
            )
        })?;
        let gateway = written(|out| {
            failure(
                out,
                &AgentError::Upstream("127.0.0.1:8079: refused".to_string()),
            )
        })?;
        let gone = written(|out| failure(out, &AgentError::from(std::io::Error::other("reset"))))?;
        assert!(bad.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(
            bad.ends_with(r#"{"error":{"message":"bad request: the body must be a JSON object"}}"#)
        );
        assert!(gateway.starts_with("HTTP/1.1 502 Bad Gateway"));
        assert_eq!(gone, "");
        Ok(())
    }

    #[test]
    fn preflight_allows_the_key_headers() -> Outcome {
        let text = written(preflight)?;
        assert!(text.starts_with("HTTP/1.1 204 No Content"));
        assert!(text.contains("Access-Control-Allow-Headers: Authorization, Content-Type, x-api-key, anthropic-version"));
        Ok(())
    }

    #[test]
    fn events_follow_the_event_stream_header() -> Outcome {
        let streamed = written(|out| {
            start_events(out)?;
            send_event(out, DONE)
        });
        let text = streamed?;
        assert!(text.contains("Content-Type: text/event-stream"));
        assert!(text.ends_with("\r\n\r\ndata: [DONE]\n\n"));
        Ok(())
    }
}
