//! Calling the model server and the search engine on localhost over plain
//! HTTP/1.1, with streamed (`text/event-stream`) and chunked bodies.

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

use crate::{
    error::{AgentError, Outcome},
    response::CORS,
};

/// How long an upstream call may stay silent before it is given up on.
const TIMEOUT: Duration = Duration::from_secs(600);

/// HTTP's "OK".
const HTTP_OK: u16 = 200;

/// Assumed when an upstream response doesn't say what it is.
const DEFAULT_CONTENT_TYPE: &str = "application/json";

/// A server this one calls: its address and the `Authorization` value it
/// expects, if any.
#[derive(Debug, Clone)]
pub struct Endpoint {
    address: String,
    authorization: Option<String>,
}

/// A response from an upstream server, returned as soon as its headers have
/// arrived so a streamed body can be read as it comes.
pub struct UpstreamResponse {
    /// The numeric status, such as 200.
    pub status: u16,
    /// The `Content-Type` header, or `application/json` when there is none.
    pub content_type: String,
    /// The body, with chunked transfer encoding already removed.
    pub body: Box<dyn BufRead + Send>,
}

impl Endpoint {
    /// An endpoint at `address` (`host:port`), sending `authorization` when given.
    #[must_use]
    pub fn new(address: String, authorization: Option<String>) -> Self {
        Endpoint {
            address,
            authorization,
        }
    }

    /// The `host:port` it is reached at.
    #[must_use]
    pub fn address(&self) -> &str {
        &self.address
    }

    /// The `Authorization` value sent with each request.
    #[cfg(test)]
    #[must_use]
    pub fn authorization(&self) -> Option<&str> {
        self.authorization.as_deref()
    }

    /// `GET path`.
    ///
    /// # Errors
    ///
    /// `AgentError::Upstream` when the server can't be reached or sends no
    /// status line; `AgentError::Io` when the connection fails midway.
    pub fn get(&self, path: &str) -> Outcome<UpstreamResponse> {
        self.send("GET", path, &[])
    }

    /// `POST path` with a JSON `body`.
    ///
    /// # Errors
    ///
    /// As for [`Endpoint::get`].
    pub fn post(&self, path: &str, body: &[u8]) -> Outcome<UpstreamResponse> {
        self.send("POST", path, body)
    }

    fn send(&self, method: &str, path: &str, body: &[u8]) -> Outcome<UpstreamResponse> {
        let mut stream = TcpStream::connect(&self.address)
            .map_err(|error| AgentError::Upstream(format!("{}: {error}", self.address)))?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        let authorization = self
            .authorization
            .as_ref()
            .map_or(String::new(), |value| format!("Authorization: {value}\r\n"));
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\n{authorization}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.address,
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(body)?;
        read_response(Box::new(BufReader::new(stream)), &self.address)
    }
}

impl UpstreamResponse {
    /// Whether the server answered "200 OK".
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.status == HTTP_OK
    }

    /// Reads the whole body as text.
    ///
    /// # Errors
    ///
    /// `AgentError::Io` when the connection fails or the body isn't UTF-8.
    pub fn text(mut self) -> Outcome<String> {
        let mut text = String::new();
        self.body.read_to_string(&mut text)?;
        Ok(text)
    }

    /// Writes this response to a client as it arrives: status, content type
    /// and body, streamed or not.
    ///
    /// # Errors
    ///
    /// `AgentError::Io` when either side's connection fails.
    pub fn relay(mut self, client: &mut dyn Write) -> Outcome {
        write!(
            client,
            "HTTP/1.1 {} Upstream\r\n{CORS}Content-Type: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
            self.status, self.content_type
        )?;
        io::copy(&mut self.body, client)?;
        Ok(client.flush()?)
    }
}

/// Reads a status line and headers from `reader`, leaving it at the body.
fn read_response(mut reader: Box<dyn BufRead + Send>, address: &str) -> Outcome<UpstreamResponse> {
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    let Some(status) = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
    else {
        return Err(AgentError::Upstream(format!("{address}: no status line")));
    };
    let mut chunked = false;
    let mut content_type = DEFAULT_CONTENT_TYPE.to_string();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim().to_ascii_lowercase();
        if header.is_empty() {
            break;
        }
        chunked |= header.starts_with("transfer-encoding:") && header.contains("chunked");
        if let Some(value) = header.strip_prefix("content-type:") {
            content_type = value.trim().to_string();
        }
    }
    let body: Box<dyn BufRead + Send> = if chunked {
        Box::new(BufReader::new(Dechunk::new(reader)))
    } else {
        Box::new(reader)
    };
    Ok(UpstreamResponse {
        status,
        content_type,
        body,
    })
}

/// Removes HTTP chunked transfer encoding from a body as it is read.
struct Dechunk {
    inner: Box<dyn BufRead + Send>,
    remaining: usize,
    done: bool,
}

impl Dechunk {
    fn new(inner: Box<dyn BufRead + Send>) -> Self {
        Dechunk {
            inner,
            remaining: 0,
            done: false,
        }
    }

    /// Reads the next chunk's size line; `false` once the body is over.
    fn next_chunk(&mut self) -> Outcome<bool> {
        if self.done {
            return Ok(false);
        }
        let mut line = String::new();
        let size = match self.inner.read_line(&mut line)? {
            0 => 0,
            _ => chunk_size(&line)
                .ok_or_else(|| AgentError::Upstream("bad chunk size".to_string()))?,
        };
        self.remaining = size;
        self.done = size == 0;
        Ok(!self.done)
    }
}

impl Read for Dechunk {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 && !self.next_chunk().map_err(io::Error::other)? {
            return Ok(0);
        }
        let wanted = buffer.len().min(self.remaining);
        let read = self.inner.read(&mut buffer[..wanted])?;
        self.remaining -= read;
        if self.remaining == 0 {
            self.inner.read_line(&mut String::new())?;
        }
        Ok(read)
    }
}

/// The size in a chunk's header line, hexadecimal, before any `;extension`.
fn chunk_size(line: &str) -> Option<usize> {
    let digits = line.trim().split(';').next().unwrap_or_default();
    usize::from_str_radix(digits, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dechunks_in_small_reads_and_stays_ended() -> Outcome {
        let mut body = Dechunk::new(Box::new(std::io::Cursor::new(
            "3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n",
        )));
        let mut buffer = [0_u8; 3];
        let sizes = [
            body.read(&mut buffer)?,
            body.read(&mut buffer)?,
            body.read(&mut buffer)?,
            body.read(&mut buffer)?,
        ];
        assert_eq!(sizes, [3, 2, 0, 0]);
        let mut partial = Dechunk::new(Box::new(std::io::Cursor::new("5\r\nhello\r\n0\r\n\r\n")));
        let mut small = [0_u8; 2];
        assert_eq!(
            [
                partial.read(&mut small)?,
                partial.read(&mut small)?,
                partial.read(&mut small)?
            ],
            [2, 2, 1]
        );
        Ok(())
    }

    #[test]
    fn a_gone_client_stops_the_relay() -> Outcome {
        let server = crate::testing::FakeServer::start(vec![
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nok"
                .to_string(),
        ])?;
        let response = Endpoint::new(server.address(), None).get("/")?;
        assert!(response.relay(&mut crate::testing::Gone).is_err());
        Ok(())
    }
    use crate::testing::FakeServer;
    use std::io::Cursor;

    fn parsed(raw: &'static str) -> Outcome<UpstreamResponse> {
        read_response(Box::new(Cursor::new(raw.as_bytes().to_vec())), "test")
    }

    #[test]
    fn removes_chunked_encoding() -> Outcome {
        let response = parsed(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;x=1\r\n world\r\n0\r\n\r\n",
        )?;
        assert_eq!(response.text()?, "hello world");
        Ok(())
    }

    #[test]
    fn reads_status_and_content_type() -> Outcome {
        let response = parsed("HTTP/1.1 404 Not Found\r\nContent-Type: Text/Plain\r\n\r\nnope")?;
        assert!(!response.is_ok());
        assert_eq!(
            (response.status, response.content_type.as_str()),
            (404, "text/plain")
        );
        assert_eq!(response.text()?, "nope");
        Ok(())
    }

    #[test]
    fn content_type_defaults_to_json() -> Outcome {
        assert_eq!(
            parsed("HTTP/1.1 200 OK\r\n\r\n{}")?.content_type,
            "application/json"
        );
        Ok(())
    }

    #[test]
    fn a_missing_status_line_is_an_upstream_error() {
        assert!(
            parsed("garbage")
                .is_err_and(|error| error.to_string() == "upstream: test: no status line")
        );
    }

    #[test]
    fn a_bad_chunk_size_is_an_error() -> Outcome {
        let response = parsed("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n")?;
        assert!(response.text().is_err());
        Ok(())
    }

    #[test]
    fn a_chunked_body_cut_short_ends_cleanly() -> Outcome {
        let response = parsed("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")?;
        assert_eq!(response.text()?, "");
        Ok(())
    }

    #[test]
    fn relay_copies_status_type_and_body() -> Outcome {
        let mut client = Vec::new();
        parsed("HTTP/1.1 201 Created\r\nContent-Type: text/event-stream\r\n\r\ndata: x\n\n")?
            .relay(&mut client)?;
        let text = String::from_utf8_lossy(&client);
        assert!(text.starts_with("HTTP/1.1 201 Upstream\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: text/event-stream"));
        assert!(text.ends_with("\r\n\r\ndata: x\n\n"));
        Ok(())
    }

    #[test]
    fn sends_method_path_key_and_body() -> Outcome {
        let server = FakeServer::start(vec!["HTTP/1.1 200 OK\r\n\r\nok".to_string()])?;
        let endpoint = Endpoint::new(server.address(), Some("Bearer k".to_string()));
        assert_eq!(endpoint.post("/v1/x", b"{}")?.text()?, "ok");
        let seen = server.requests()?;
        assert!(seen[0].starts_with("POST /v1/x HTTP/1.1\r\n"));
        assert!(seen[0].contains("Authorization: Bearer k\r\n"));
        assert!(seen[0].ends_with("\r\n\r\n{}"));
        Ok(())
    }

    #[test]
    fn an_unreachable_server_is_an_upstream_error() {
        let endpoint = Endpoint::new("127.0.0.1:1".to_string(), None);
        assert!(
            endpoint
                .get("/")
                .is_err_and(|error| error.to_string().starts_with("upstream: 127.0.0.1:1:"))
        );
    }
}
