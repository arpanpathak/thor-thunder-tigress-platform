//! Just enough HTTP for one page and its API.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
};

use crate::error::ReviewError;

/// The largest request body accepted, in bytes.
const MAX_BODY: usize = 1 << 20;

/// One parsed request.
pub struct Request {
    /// The method, uppercased.
    pub method: String,
    /// The path, without the query string.
    pub path: String,
    /// The decoded query parameters, in order.
    pub query: Vec<(String, String)>,
    /// The body, empty for a request without one.
    pub body: String,
}

impl Request {
    /// The first value of a parameter.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// A parameter as a number, or `fallback` when absent or unparsable.
    pub fn number(&self, name: &str, fallback: usize) -> usize {
        self.param(name)
            .and_then(|value| value.parse().ok())
            .unwrap_or(fallback)
    }

    /// A parameter as a flag: `1`, `true`, `yes` are true; `0`, `false`, `no` are false.
    pub fn boolean(&self, name: &str) -> Option<bool> {
        self.param(name).and_then(|value| match value {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        })
    }
}

/// Reads one request, consuming the headers so the socket closes cleanly.
pub fn read_request(stream: &mut TcpStream) -> Result<Request, ReviewError> {
    let mut reader = BufReader::new(stream.try_clone().map_err(stream_error)?);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(stream_error)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_uppercase();
    let target = parts.next().unwrap_or_default().to_string();
    let (path, query) = split_target(&target);
    let content_length = read_headers(&mut reader)?;
    let body = read_body(&mut reader, content_length)?;
    Ok(Request {
        method,
        path,
        query,
        body,
    })
}

/// Splits a request target into its path and decoded parameters.
fn split_target(target: &str) -> (String, Vec<(String, String)>) {
    match target.split_once('?') {
        None => (target.to_string(), Vec::new()),
        Some((path, query)) => (path.to_string(), parse_query(query)),
    }
}

/// Decodes `key=value` pairs joined by `&`.
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => (decode(key), decode(value)),
            None => (decode(pair), String::new()),
        })
        .collect()
}

/// Decodes percent escapes and `+`.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut position = 0usize;
    while position < bytes.len() {
        match bytes[position] {
            b'+' => {
                out.push(b' ');
                position += 1;
            }
            b'%' if position + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[position + 1..position + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        position += 3;
                    }
                    Err(_) => {
                        out.push(bytes[position]);
                        position += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                position += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Reads the headers, returning `Content-Length`.
fn read_headers(reader: &mut BufReader<TcpStream>) -> Result<usize, ReviewError> {
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).map_err(stream_error)?;
        if read == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0).min(MAX_BODY);
        }
    }
    Ok(content_length)
}

/// Reads a body of the given length.
fn read_body(reader: &mut BufReader<TcpStream>, length: usize) -> Result<String, ReviewError> {
    if length == 0 {
        return Ok(String::new());
    }
    let mut buffer = vec![0u8; length];
    reader.read_exact(&mut buffer).map_err(stream_error)?;
    Ok(String::from_utf8_lossy(&buffer).to_string())
}

/// Writes a complete response and closes the connection.
pub fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), ReviewError> {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).map_err(stream_error)?;
    stream.write_all(body).map_err(stream_error)?;
    stream.flush().map_err(stream_error)
}

/// Writes a JSON response.
pub fn write_json(stream: &mut TcpStream, status: &str, value: &serde_json::Value) -> Result<(), ReviewError> {
    let body = serde_json::to_vec(value).map_err(|error| ReviewError::BadRequest(error.to_string()))?;
    write_response(stream, status, "application/json; charset=utf-8", &body)
}

/// A converter for `map_err` on a socket.
fn stream_error(source: std::io::Error) -> ReviewError {
    ReviewError::BadRequest(source.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_escapes_and_plus() {
        assert_eq!(decode("page%20table"), "page table");
        assert_eq!(decode("a+b"), "a b");
        assert_eq!(decode("plain"), "plain");
    }

    #[test]
    fn parses_a_query_string() {
        let query = parse_query("start=20&q=page%20table&flagged=1");
        assert_eq!(query.len(), 3);
        assert_eq!(query[1].1, "page table");
    }

    #[test]
    fn splits_path_from_query() {
        let (path, query) = split_target("/api/page?start=0");
        assert_eq!(path, "/api/page");
        assert_eq!(query.len(), 1);
    }
}
