//! Reading one HTTP/1.1 request from a client: the request line, the two
//! headers this server cares about, and the body.

use std::io::{BufRead, BufReader, Read};

use crate::{
    config::BEARER,
    error::{AgentError, Outcome},
};

/// The largest request body accepted: a long conversation with pasted code.
const MAX_BODY: usize = 32 << 20;

/// One request from a client.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    /// The method, such as `GET`.
    pub method: String,
    /// The path without the query string.
    pub path: String,
    /// The `Authorization` header; an `x-api-key` header (Anthropic clients)
    /// becomes `Bearer <key>`.
    pub authorization: Option<String>,
    /// The body.
    pub body: Vec<u8>,
}

/// The headers this server reads.
#[derive(Default)]
struct Headers {
    content_length: usize,
    authorization: Option<String>,
}

/// Reads one request from `stream`.
///
/// # Errors
///
/// `AgentError::Io` when the connection fails, `AgentError::BadRequest` for
/// a bad `Content-Length` or a body over 32 MiB.
pub fn read_request(stream: impl Read) -> Outcome<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default();
    let path = target.split_once('?').map_or(target, |(path, _)| path).to_string();
    let headers = read_headers(&mut reader)?;
    let body = read_body(&mut reader, headers.content_length)?;
    Ok(Request {
        method,
        path,
        authorization: headers.authorization,
        body,
    })
}

fn read_headers(reader: &mut impl BufRead) -> Outcome<Headers> {
    let mut headers = Headers::default();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            return Ok(headers);
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.note(&name.trim().to_ascii_lowercase(), value.trim())?;
    }
}

impl Headers {
    fn note(&mut self, name: &str, value: &str) -> Outcome {
        match name {
            "content-length" => {
                self.content_length = value
                    .parse()
                    .map_err(|_| AgentError::bad_request("bad content-length"))?;
            }
            "authorization" => self.authorization = Some(value.to_string()),
            "x-api-key" => self.authorization = Some(format!("{BEARER}{value}")),
            _ => {}
        }
        Ok(())
    }
}

fn read_body(reader: &mut impl Read, length: usize) -> Outcome<Vec<u8>> {
    if length > MAX_BODY {
        return Err(AgentError::bad_request(format!("body over {MAX_BODY} bytes")));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Outcome<Request> {
        read_request(raw.as_bytes())
    }

    #[test]
    fn reads_method_path_and_body() -> Outcome {
        let request = parse("POST /v1/chat/completions?x=1 HTTP/1.1\r\nHost: t\r\nContent-Length: 2\r\n\r\n{}")?;
        assert_eq!(
            request,
            Request {
                method: "POST".to_string(),
                path: "/v1/chat/completions".to_string(),
                authorization: None,
                body: b"{}".to_vec(),
            }
        );
        Ok(())
    }

    #[test]
    fn header_names_are_case_insensitive() -> Outcome {
        let request = parse("GET / HTTP/1.1\r\nauthorization: Bearer k\r\n\r\n")?;
        assert_eq!(request.authorization.as_deref(), Some("Bearer k"));
        Ok(())
    }

    #[test]
    fn x_api_key_becomes_a_bearer_key() -> Outcome {
        let request = parse("POST /v1/messages HTTP/1.1\r\nX-Api-Key: k\r\n\r\n")?;
        assert_eq!(request.authorization.as_deref(), Some("Bearer k"));
        Ok(())
    }

    #[test]
    fn skips_lines_that_are_not_headers() -> Outcome {
        let request = parse("GET /health HTTP/1.1\r\nnonsense\r\n\r\n")?;
        assert_eq!(request.path, "/health");
        Ok(())
    }

    #[test]
    fn rejects_bad_and_huge_lengths() {
        let bad = parse("POST / HTTP/1.1\r\nContent-Length: ten\r\n\r\n");
        let huge = parse("POST / HTTP/1.1\r\nContent-Length: 999999999\r\n\r\n");
        assert!(bad.is_err_and(|error| error.to_string() == "bad request: bad content-length"));
        assert!(huge.is_err_and(|error| error.to_string().starts_with("bad request: body over")));
    }

    #[test]
    fn a_short_body_is_an_error() {
        assert!(parse("POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\nab").is_err());
    }

    #[test]
    fn an_empty_connection_reads_as_an_empty_request() -> Outcome {
        let request = parse("")?;
        assert_eq!((request.method.as_str(), request.path.as_str()), ("", ""));
        Ok(())
    }
}
