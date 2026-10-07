//! Just enough HTTP for one page and its API: reading a request with its query
//! parameters and body, and writing an answer.

use std::{
    io::{BufRead, BufReader, Read, Write},
    str::Bytes,
};

use serde::Serialize;

use crate::error::{Outcome, ReviewError};

/// The largest request body accepted, in bytes.
const MAX_BODY: usize = 1 << 20;

/// The statuses this tool answers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 200.
    Ok,
    /// 400: the request is malformed.
    BadRequest,
    /// 404: no such page or record.
    NotFound,
    /// 500: a file failed, or the flags are unavailable.
    InternalError,
}

/// The kinds of body this tool sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    /// The review page.
    Html,
    /// The API's answers.
    Json,
    /// The removed examples, one JSON object per line.
    JsonLines,
}

impl Status {
    fn line(self) -> &'static str {
        match self {
            Status::Ok => "200 OK",
            Status::BadRequest => "400 Bad Request",
            Status::NotFound => "404 Not Found",
            Status::InternalError => "500 Internal Server Error",
        }
    }
}

impl ContentType {
    fn header(self) -> &'static str {
        match self {
            ContentType::Html => "text/html; charset=utf-8",
            ContentType::Json => "application/json; charset=utf-8",
            ContentType::JsonLines => "application/x-ndjson; charset=utf-8",
        }
    }
}

/// One `key=value` from the query string, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Param {
    key: String,
    value: String,
}

/// One parsed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The method, uppercased.
    pub method: String,
    /// The path, without the query string.
    pub path: String,
    /// The query parameters, in order.
    params: Vec<Param>,
    /// The body, empty for a request without one.
    pub body: String,
}

impl Request {
    /// A request built by hand, for tests.
    #[cfg(test)]
    pub fn new(method: &str, target: &str, body: &str) -> Request {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        Request {
            method: method.to_string(),
            path: path.to_string(),
            params: parse_query(query),
            body: body.to_string(),
        }
    }

    /// The first value of a parameter.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|param| param.key == name)
            .map(|param| param.value.as_str())
    }

    /// A parameter that counts only when it has text.
    #[must_use]
    pub fn text(&self, name: &str) -> Option<String> {
        self.param(name)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_string)
    }

    /// A parameter as a number, or `fallback` when absent or unparsable.
    #[must_use]
    pub fn number(&self, name: &str, fallback: usize) -> usize {
        self.param(name)
            .and_then(|value| value.parse().ok())
            .unwrap_or(fallback)
    }

    /// A parameter as a yes or no: `1`, `true`, `yes` or `0`, `false`, `no`.
    #[must_use]
    pub fn boolean(&self, name: &str) -> Option<bool> {
        match self.param(name)? {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        }
    }
}

/// Reads one request.
///
/// # Errors
///
/// `ReviewError::Connection` when the connection fails, `ReviewError::BadRequest`
/// for a body over 1 MiB.
pub fn read_request(stream: &mut dyn Read) -> Outcome<Request> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_uppercase();
    let target = parts.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let length = read_content_length(&mut reader)?;

    if length > MAX_BODY {
        return Err(ReviewError::BadRequest(format!(
            "body over {MAX_BODY} bytes"
        )));
    }

    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        method,
        path: path.to_string(),
        params: parse_query(query),
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Reads the headers up to the blank line, returning `Content-Length`.
fn read_content_length(reader: &mut impl BufRead) -> Outcome<usize> {
    let mut length = 0;

    loop {
        let mut line = String::new();

        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            return Ok(length);
        }

        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
}

/// Decodes `key=value` pairs joined by `&`.
fn parse_query(query: &str) -> Vec<Param> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Param {
                key: decode(key),
                value: decode(value),
            }
        })
        .collect()
}

/// Decodes `+` and `%XY` escapes; a `%` not followed by two hex digits stays.
fn decode(text: &str) -> String {
    let mut bytes = text.bytes();
    let mut out = Vec::with_capacity(text.len());

    while let Some(byte) = bytes.next() {
        match byte {
            b'+' => out.push(b' '),
            b'%' => out.push(percent_escape(&mut bytes).unwrap_or(b'%')),
            other => out.push(other),
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

/// The byte a `%XY` escape stands for, consuming `XY` only when both are hex digits.
fn percent_escape(bytes: &mut Bytes<'_>) -> Option<u8> {
    let mut ahead = bytes.clone();
    let high = hex_digit(ahead.next()?)?;
    let low = hex_digit(ahead.next()?)?;
    *bytes = ahead;
    Some(high * 16 + low)
}

fn hex_digit(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

/// Writes a complete response and closes the connection.
///
/// # Errors
///
/// `ReviewError::Connection` when the client is gone.
pub fn write_response(
    stream: &mut dyn Write,
    status: Status,
    content_type: ContentType,
    body: &[u8],
) -> Outcome {
    let head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status.line(),
        content_type.header(),
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    Ok(stream.flush()?)
}

/// Writes `value` as a JSON response.
///
/// # Errors
///
/// `ReviewError::Connection` when the client is gone.
pub fn write_json(stream: &mut dyn Write, status: Status, value: &impl Serialize) -> Outcome {
    let body = serde_json::to_vec(value).map_err(ReviewError::unserializable)?;
    write_response(stream, status, ContentType::Json, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value whose serialization always fails.
    struct Unserializable;

    impl Serialize for Unserializable {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("refused"))
        }
    }

    #[test]
    fn a_value_that_cannot_be_serialized_is_a_bad_request() {
        let mut out = Vec::new();
        let result = write_json(&mut out, Status::Ok, &Unserializable);
        assert!(
            matches!(result, Err(ReviewError::BadRequest(message)) if message.contains("refused"))
        );
        assert!(out.is_empty());
    }

    #[test]
    fn decodes_percent_escapes_and_plus() {
        assert_eq!(decode("page%20table"), "page table");
        assert_eq!(decode("a+b"), "a b");
        assert_eq!(decode("caf%C3%A9"), "café");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz%4"), "%zz%4");
        assert_eq!(decode("%+1"), "% 1");
    }

    #[test]
    fn reads_method_path_params_and_body() -> Outcome {
        let request = read_request(&mut "post /api/page?start=20&q=page%20table&flagged=1&empty= HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}".as_bytes())?;
        assert_eq!(
            (
                request.method.as_str(),
                request.path.as_str(),
                request.body.as_str()
            ),
            ("POST", "/api/page", "{}")
        );
        assert_eq!(request.param("q"), Some("page table"));
        assert_eq!(request.number("start", 0), 20);
        assert_eq!(request.number("limit", 7), 7);
        assert_eq!(request.boolean("flagged"), Some(true));
        assert_eq!(request.text("empty"), None);
        Ok(())
    }

    #[test]
    fn reads_yes_and_no_in_their_spellings() {
        let request = Request::new("GET", "/?a=yes&b=no&c=false&d=maybe", "");
        let answers = ["a", "b", "c", "d", "e"].map(|name| request.boolean(name));
        assert_eq!(answers, [Some(true), Some(false), Some(false), None, None]);
    }

    #[test]
    fn rejects_a_body_over_the_limit() {
        let raw = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        assert!(
            read_request(&mut raw.as_bytes())
                .is_err_and(|error| error.to_string().starts_with("bad request: body over"))
        );
    }

    #[test]
    fn writes_status_type_and_json() -> Outcome {
        let mut out = Vec::new();
        write_json(
            &mut out,
            Status::NotFound,
            &serde_json::json!({ "error": "x" }),
        )?;
        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with(
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json; charset=utf-8\r\n"
        ));
        assert!(text.ends_with(r#"{"error":"x"}"#));
        Ok(())
    }

    #[test]
    fn every_status_and_type_has_its_header() {
        let lines = [
            Status::Ok,
            Status::BadRequest,
            Status::NotFound,
            Status::InternalError,
        ]
        .map(Status::line);
        let types =
            [ContentType::Html, ContentType::Json, ContentType::JsonLines].map(ContentType::header);
        assert_eq!(
            lines,
            [
                "200 OK",
                "400 Bad Request",
                "404 Not Found",
                "500 Internal Server Error"
            ]
        );
        assert_eq!(types[2], "application/x-ndjson; charset=utf-8");
    }
}
