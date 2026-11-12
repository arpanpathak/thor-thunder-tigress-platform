//! One line of JSON per request, written to the request log when the answer
//! ends: who asked, from where, the route, the model, the status, the time to
//! the first byte and to the last, the bytes sent, and the tokens the engine
//! reported.
//!
//! The answer is watched as it streams to the client, so a long reply is not
//! held back. Token counts are read from the `usage` the engines put in their
//! answers (OpenAI's `prompt_tokens` and `completion_tokens`, Anthropic's
//! `input_tokens` and `output_tokens`), keeping the largest value seen.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{Body, HttpBody},
    extract::Request,
    http::{HeaderMap, HeaderValue, header},
    response::Response,
};
use serde::Serialize;
use tokio_stream::StreamExt;

use crate::{error::Outcome, server::limits::Permit};

/// The JSON names that carry the prompt's token count.
const INPUT_NAMES: [&str; 2] = ["\"prompt_tokens\"", "\"input_tokens\""];

/// The JSON names that carry the answer's token count.
const OUTPUT_NAMES: [&str; 2] = ["\"completion_tokens\"", "\"output_tokens\""];

/// How many bytes of a chunk are kept to read a name split across two chunks.
const CARRY: usize = 64;

/// The file the records are appended to.
pub struct RequestLog {
    file: Mutex<File>,
}

/// One request, as written to the log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Record {
    /// When the request arrived, in milliseconds since 1970.
    pub at_ms: u64,
    /// The person the key belongs to, or `anonymous`.
    pub who: String,
    /// The HTTP method.
    pub method: String,
    /// The route, such as `/v1/chat/completions`.
    pub path: String,
    /// The client's address, the first one in `X-Forwarded-For`.
    pub client: Option<String>,
    /// The client's `User-Agent`.
    pub user_agent: Option<String>,
    /// The model the request named, if any.
    pub model: Option<String>,
    /// The HTTP status of the answer.
    pub status: u16,
    /// Whether the person's limits refused it.
    pub limited: bool,
    /// Milliseconds from arrival to the first byte of the answer.
    pub first_byte_ms: u64,
    /// Milliseconds from arrival to the end of the answer.
    pub total_ms: u64,
    /// Bytes of answer sent.
    pub bytes: u64,
    /// Tokens in the prompt, as the engine reported them.
    pub input_tokens: u64,
    /// Tokens in the answer, as the engine reported them.
    pub output_tokens: u64,
}

impl RequestLog {
    /// Opens `path` for appending, creating it and its folder when missing.
    ///
    /// # Errors
    ///
    /// The I/O error when the folder or the file cannot be made or opened.
    pub fn open(path: &Path) -> Outcome<Self> {
        let folder = path.parent().unwrap_or(Path::new("."));

        std::fs::create_dir_all(folder)?;

        let file = OpenOptions::new().create(true).append(true).open(path)?;

        Ok(RequestLog {
            file: Mutex::new(file),
        })
    }

    /// Appends `record` as one line. A failed write is reported, not raised:
    /// the client already has its answer.
    pub fn write(&self, record: &Record) {
        let line = serde_json::to_string(record).unwrap_or_default() + "\n";
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if let Err(error) = file.write_all(line.as_bytes()) {
            eprintln!("request log: {error}");
        }
    }
}

impl Record {
    /// A record for a request from `who` to `path` naming `model`, arriving now.
    #[must_use]
    pub fn new(who: &str, path: &str, model: Option<String>) -> Self {
        Record {
            at_ms: unix_ms(SystemTime::now()),
            who: who.to_string(),
            path: path.to_string(),
            model,
            ..Record::default()
        }
    }

    /// A record for `request` from `who`, arriving now.
    #[must_use]
    pub fn of(request: &Request, who: &str) -> Self {
        let headers = request.headers();

        Record {
            method: request.method().to_string(),
            client: forwarded_for(headers),
            user_agent: text(headers, header::USER_AGENT.as_str()),
            ..Record::new(who, request.uri().path(), None)
        }
    }
}

/// The first address in `X-Forwarded-For`: the client, before any proxy.
fn forwarded_for(headers: &HeaderMap) -> Option<String> {
    let list = text(headers, "x-forwarded-for")?;
    let first = list.split(',').next()?.trim();

    Some(first.to_string())
}

fn text(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;

    Some(value.to_string())
}

/// What is known of an answer while it streams, written to the log when the
/// stream ends or the client goes away.
struct Tally {
    record: Record,
    started: Instant,
    first_byte: Option<Instant>,
    carried: Vec<u8>,
    log: Option<Arc<RequestLog>>,
    _permit: Option<Permit>,
}

impl Tally {
    fn see(&mut self, chunk: &[u8]) {
        self.first_byte.get_or_insert_with(Instant::now);
        self.record.bytes += chunk.len() as u64;

        let mut window = std::mem::take(&mut self.carried);
        window.extend_from_slice(chunk);

        let text = String::from_utf8_lossy(&window);
        self.record.input_tokens = self.record.input_tokens.max(largest(&text, &INPUT_NAMES));
        self.record.output_tokens = self.record.output_tokens.max(largest(&text, &OUTPUT_NAMES));

        self.carried = window[window.len().saturating_sub(CARRY)..].to_vec();
    }
}

impl Drop for Tally {
    fn drop(&mut self) {
        let first = self.first_byte.unwrap_or_else(Instant::now);
        self.record.first_byte_ms = millis(first.saturating_duration_since(self.started));
        self.record.total_ms = millis(self.started.elapsed());

        if let Some(log) = &self.log {
            log.write(&self.record);
        }
    }
}

/// Passes `response` on unchanged, and writes `record` to `log` once its body
/// has been sent. `permit` is held until then. A body whose length is known
/// keeps its `Content-Length`, so only a streamed answer is sent in chunks.
pub fn metered(
    response: Response,
    mut record: Record,
    started: Instant,
    log: Option<Arc<RequestLog>>,
    permit: Option<Permit>,
) -> Response {
    record.status = response.status().as_u16();

    let mut tally = Tally {
        record,
        started,
        first_byte: None,
        carried: Vec::new(),
        log,
        _permit: permit,
    };
    let length = response.body().size_hint().exact();
    let (mut parts, body) = response.into_parts();
    keep_length(&mut parts.headers, length);

    let watched = body.into_data_stream().map(move |chunk| {
        if let Ok(bytes) = &chunk {
            tally.see(bytes);
        }

        chunk
    });

    Response::from_parts(parts, Body::from_stream(watched))
}

fn keep_length(headers: &mut HeaderMap, length: Option<u64>) {
    if let Some(length) = length {
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    }
}

/// Writes a record for a request that got no answer from an engine, such as
/// one the limits refused.
pub fn note(log: Option<&RequestLog>, record: &Record) {
    if let Some(log) = log {
        log.write(record);
    }
}

/// The largest number following any of `names` in `text`, or 0.
fn largest(text: &str, names: &[&str]) -> u64 {
    names
        .iter()
        .flat_map(|name| {
            text.match_indices(name)
                .map(|(at, _)| &text[at + name.len()..])
        })
        .filter_map(number_after_colon)
        .max()
        .unwrap_or(0)
}

/// The number in `rest` once a colon and spaces are skipped: `": 12,"` is 12.
fn number_after_colon(rest: &str) -> Option<u64> {
    let value = rest.trim_start().strip_prefix(':')?.trim_start();
    let digits = value.split(|c: char| !c.is_ascii_digit()).next()?;

    digits.parse().ok()
}

fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn unix_ms(time: SystemTime) -> u64 {
    millis(time.duration_since(UNIX_EPOCH).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};

    use super::*;
    use crate::server::limits::Limits;

    #[test]
    fn reads_the_largest_count_after_each_name() {
        let openai = r#"{"usage":{"prompt_tokens":91,"completion_tokens":15}}"#;
        let anthropic =
            r#"{"usage": {"input_tokens" : 7, "output_tokens": 1}} {"usage":{"output_tokens":42}}"#;

        assert_eq!(largest(openai, &INPUT_NAMES), 91);
        assert_eq!(largest(openai, &OUTPUT_NAMES), 15);
        assert_eq!(largest(anthropic, &INPUT_NAMES), 7);
        assert_eq!(largest(anthropic, &OUTPUT_NAMES), 42);
        assert_eq!(largest("no usage here", &OUTPUT_NAMES), 0);
        assert_eq!(largest(r#""output_tokens" 5"#, &OUTPUT_NAMES), 0);
    }

    #[tokio::test]
    async fn writes_one_record_when_the_answer_ends() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("logs/requests.jsonl");
        let log = Arc::new(RequestLog::open(&path)?);
        let limits = Limits::new(10, 1);
        let permit = limits.admit("ada", Instant::now()).ok();
        let answer =
            r#"data: {"usage":{"prompt_tok"#.to_string() + r#"ens":12,"completion_tokens":30}}"#;
        let response = (StatusCode::OK, answer.clone()).into_response();
        let record = Record::new("ada", "/v1/chat/completions", Some("gemma".to_string()));

        let metered = metered(
            response,
            record,
            Instant::now(),
            Some(Arc::clone(&log)),
            permit,
        );

        assert_eq!(limits.running("ada"), 1);

        let body = to_bytes(metered.into_body(), usize::MAX).await?;
        let lines = std::fs::read_to_string(&path)?;
        let written: serde_json::Value = serde_json::from_str(lines.trim())?;

        assert_eq!(body, answer.as_bytes());
        assert_eq!(lines.lines().count(), 1);
        assert_eq!(written["who"], "ada");
        assert_eq!(written["model"], "gemma");
        assert_eq!(written["status"], 200);
        assert_eq!(written["bytes"], answer.len());
        assert_eq!(written["input_tokens"], 12);
        assert_eq!(written["output_tokens"], 30);
        assert_eq!(limits.running("ada"), 0);

        Ok(())
    }

    #[test]
    fn a_name_split_across_chunks_is_still_read() {
        let mut tally = Tally {
            record: Record::default(),
            started: Instant::now(),
            first_byte: None,
            carried: Vec::new(),
            log: None,
            _permit: None,
        };

        let first = br#"{"usage":{"completion_to"#;
        let second = br#"kens": 77}}"#;

        tally.see(first);
        tally.see(second);

        assert_eq!(tally.record.output_tokens, 77);
        assert_eq!(tally.record.bytes, (first.len() + second.len()) as u64);
    }

    #[test]
    fn note_writes_a_record_only_when_there_is_a_log() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("requests.jsonl");
        let log = RequestLog::open(&path)?;
        let record = Record {
            limited: true,
            status: 429,
            ..Record::new("ada", "/v1/messages", None)
        };

        note(None, &record);
        note(Some(&log), &record);

        let lines = std::fs::read_to_string(&path)?;

        assert_eq!(lines.lines().count(), 1);
        assert!(lines.contains(r#""limited":true"#));

        Ok(())
    }

    #[test]
    fn a_record_of_a_request_has_its_method_client_and_agent() -> Result<()> {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/messages?beta=true")
            .header("x-forwarded-for", "203.0.113.5, 10.0.0.1")
            .header("user-agent", "curl/8.5")
            .body(Body::empty())?;
        let bare = Request::builder().uri("/").body(Body::empty())?;

        let record = Record::of(&request, "ada");
        let anonymous = Record::of(&bare, "anonymous");

        assert_eq!(record.method, "POST");
        assert_eq!(record.path, "/v1/messages");
        assert_eq!(record.client.as_deref(), Some("203.0.113.5"));
        assert_eq!(record.user_agent.as_deref(), Some("curl/8.5"));
        assert_eq!((anonymous.client, anonymous.user_agent), (None, None));

        Ok(())
    }

    #[test]
    fn a_failed_write_is_reported_not_raised() -> Result<()> {
        let full = RequestLog::open(Path::new("/dev/full"))?;

        full.write(&Record::default());

        Ok(())
    }

    #[test]
    fn a_log_that_cannot_be_opened_is_an_error() {
        assert!(RequestLog::open(Path::new("/proc/no-such-folder/requests.jsonl")).is_err());
    }
}
