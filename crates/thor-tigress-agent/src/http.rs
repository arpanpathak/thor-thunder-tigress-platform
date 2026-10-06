//! Just enough HTTP/1.1 for this server: reading a request from the browser,
//! writing responses, and calling the model server and the search engine on
//! localhost, including following a streamed (`text/event-stream`) reply.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

use crate::error::AgentError;

/// The largest request body accepted: a long conversation with pasted code.
const MAX_BODY: usize = 32 << 20;

/// Sent with every response, so the chat page can be hosted on another site
/// (GitHub Pages) and call this server. Safe because access is by bearer key,
/// not cookies: another site cannot use a visitor's key without having it.
const CORS: &str = "Access-Control-Allow-Origin: *\r\n";

/// How long an upstream call may stay silent before it is given up on.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(600);

/// One request from the browser.
pub struct Request {
    /// The method, such as `GET`.
    pub method: String,
    /// The path without the query string.
    pub path: String,
    /// The `Authorization` header, if any; an `x-api-key` header (Anthropic
    /// clients) is turned into `Bearer <key>`.
    pub authorization: Option<String>,
    /// The body.
    pub body: Vec<u8>,
}

/// Reads one request from `stream`.
pub fn read_request(stream: &TcpStream) -> Result<Request, AgentError> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default();
    let path = target.split('?').next().unwrap_or_default().to_string();
    let mut length = 0usize;
    let mut authorization = None;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                length = value
                    .trim()
                    .parse()
                    .map_err(|_| AgentError::BadRequest("bad content-length".to_string()))?
            }
            "authorization" => authorization = Some(value.trim().to_string()),
            "x-api-key" => authorization = Some(format!("Bearer {}", value.trim())),
            _ => {}
        }
    }
    if length > MAX_BODY {
        return Err(AgentError::BadRequest(format!("body over {MAX_BODY} bytes")));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        method,
        path,
        authorization,
        body,
    })
}

/// Writes a complete response.
pub fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) -> Result<(), AgentError> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\n{CORS}Content-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(stream.flush()?)
}

/// Answers a CORS preflight: the browser asks before a cross-site request
/// with an `Authorization` header.
pub fn preflight(stream: &mut TcpStream) -> Result<(), AgentError> {
    write!(
        stream,
        "HTTP/1.1 204 No Content\r\n{CORS}Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, x-api-key, anthropic-version\r\nAccess-Control-Max-Age: 86400\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )?;
    Ok(stream.flush()?)
}

/// Starts an event-stream response; events are then written with
/// [`send_event`] and the response ends when the connection closes.
pub fn start_events(stream: &mut TcpStream) -> Result<(), AgentError> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\n{CORS}Content-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    )?;
    Ok(stream.flush()?)
}

/// Writes one server-sent event carrying `data`.
pub fn send_event(stream: &mut TcpStream, data: &str) -> Result<(), AgentError> {
    write!(stream, "data: {data}\n\n")?;
    Ok(stream.flush()?)
}

/// A response from an upstream server: its status and a reader over the body,
/// with chunked transfer encoding already removed.
pub struct UpstreamResponse {
    /// The numeric status, such as 200.
    pub status: u16,
    /// The `Content-Type` header, or `application/json` when there is none.
    pub content_type: String,
    /// The body.
    pub body: Box<dyn BufRead + Send>,
}

impl UpstreamResponse {
    /// Reads the whole body as text.
    pub fn text(mut self) -> Result<String, AgentError> {
        let mut text = String::new();
        self.body.read_to_string(&mut text)?;
        Ok(text)
    }
}

/// Sends a request to `address` (`host:port`) and returns the response as
/// soon as its headers have arrived, so a streamed body can be read as it
/// comes.
pub fn call(
    address: &str,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: Option<&[u8]>,
) -> Result<UpstreamResponse, AgentError> {
    let mut stream = TcpStream::connect(address)
        .map_err(|error| AgentError::Upstream(format!("{address}: {error}")))?;
    stream.set_read_timeout(Some(UPSTREAM_TIMEOUT))?;
    let auth_header = authorization.map_or(String::new(), |value| format!("Authorization: {value}\r\n"));
    let content = body.unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\n{auth_header}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        content.len()
    )?;
    stream.write_all(content)?;
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| AgentError::Upstream(format!("{address}: no status line")))?;
    let mut chunked = false;
    let mut content_type = "application/json".to_string();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header.trim().is_empty() {
            break;
        }
        let lowered = header.to_ascii_lowercase();
        chunked |= lowered.starts_with("transfer-encoding:") && lowered.contains("chunked");
        if let Some(value) = lowered.strip_prefix("content-type:") {
            content_type = value.trim().to_string();
        }
    }
    let body: Box<dyn BufRead + Send> = match chunked {
        true => Box::new(BufReader::new(Dechunk {
            inner: reader,
            remaining: 0,
            done: false,
        })),
        false => Box::new(reader),
    };
    Ok(UpstreamResponse {
        status,
        content_type,
        body,
    })
}

/// Writes an upstream response to `stream` as it arrives: status, content
/// type and body, streamed or not.
pub fn relay(stream: &mut TcpStream, mut response: UpstreamResponse) -> Result<(), AgentError> {
    write!(
        stream,
        "HTTP/1.1 {} Upstream\r\n{CORS}Content-Type: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        response.status, response.content_type
    )?;
    std::io::copy(&mut response.body, stream)?;
    Ok(stream.flush()?)
}

/// Removes HTTP chunked transfer encoding from a body as it is read.
struct Dechunk<R: BufRead> {
    inner: R,
    remaining: usize,
    done: bool,
}

impl<R: BufRead> Read for Dechunk<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.done {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut size_line = String::new();
            if self.inner.read_line(&mut size_line)? == 0 {
                self.done = true;
                return Ok(0);
            }
            let size_text = size_line.trim().split(';').next().unwrap_or_default();
            self.remaining = usize::from_str_radix(size_text, 16)
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk size"))?;
            if self.remaining == 0 {
                self.done = true;
                return Ok(0);
            }
        }
        let wanted = buffer.len().min(self.remaining);
        let read = self.inner.read(&mut buffer[..wanted])?;
        self.remaining -= read;
        if self.remaining == 0 {
            let mut crlf = String::new();
            self.inner.read_line(&mut crlf)?;
        }
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_chunked_encoding() -> Result<(), AgentError> {
        let raw = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let mut decoded = String::new();
        Dechunk {
            inner: BufReader::new(&raw[..]),
            remaining: 0,
            done: false,
        }
        .read_to_string(&mut decoded)?;
        assert_eq!(decoded, "hello world");
        Ok(())
    }
}
