//! A local web page for reading the training set and flagging AI slop.
//!
//! ## Design
//!
//! ```text
//!   GET  /             ─► review.html, compiled into the binary
//!   GET  /train.jsonl  ─► the training file, read fresh on every request
//!   GET  /slop.jsonl   ─► examples removed from training as slop, so they stay reviewable
//!   GET  /flags        ─► labels/slop_flags.jsonl, one {id, note, spans} per line
//!   POST /flags        ─► {id, note, spans, flagged}: replace or clear one flag, rewrite the file
//! ```
//!
//! Searching, filtering and highlighting happen in the page. The server binds
//! to 127.0.0.1 only. `POST /flags` accepts only `Content-Type: application/json`:
//! a browser sends that type cross-origin only after a CORS preflight, which this
//! server never approves, so other websites open in the same browser cannot
//! write flags.
//!
//! ```text
//! cargo run --release -p thor-hammer-trainer --bin review -- [data/train.jsonl] [PORT] [labels/slop_flags.jsonl]
//! ```

use std::{
    fmt,
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::ExitCode,
};

use serde::{Deserialize, Serialize};

/// The review page, embedded at compile time so the binary needs no extra files.
const REVIEW_PAGE: &str = include_str!("review.html");

/// The training file served when no path is given.
const DEFAULT_TRAINING_FILE: &str = "data/train.jsonl";

/// The port used when no port is given.
const DEFAULT_PORT: &str = "8080";

/// The slop flags file used when no path is given.
const DEFAULT_FLAGS_FILE: &str = "labels/slop_flags.jsonl";

/// The largest request body accepted; a flag with spans is a few kilobytes.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// A failure that stops the server or one request.
#[derive(Debug)]
enum ServeError {
    /// A socket or file operation failed.
    Io {
        /// What the server was doing.
        action: String,
        /// What the operating system reported.
        source: io::Error,
    },
    /// A flag in the request or the flags file is not valid JSON.
    Json(serde_json::Error),
    /// The training file does not exist yet.
    MissingTrainingFile(PathBuf),
}

/// The files the server reads and writes.
struct Files {
    training: PathBuf,
    removed_as_slop: PathBuf,
    flags: PathBuf,
}

/// The parts of an HTTP request the server uses.
struct Request {
    method: String,
    path: String,
    content_type: String,
    body: Vec<u8>,
}

/// One HTTP response: status line, content type and body.
struct Response {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

/// The slop categories of `anti_ai_slop.md`. The same list as
/// `slop_flags::SlopCategory` in the generator; a request with any other
/// category is rejected.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum SlopCategory {
    FakeImportance,
    DramaticSetup,
    EmptyDepthWords,
    FakeBalanceHedging,
    FlatteryFillerOpener,
    WrapUpRepeat,
    RhythmTrick,
    Other,
}

/// Which part of an example a span was selected in.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Field {
    Instruction,
    Response,
}

/// One sentence or phrase marked as slop.
#[derive(Deserialize, Serialize)]
struct SlopSpan {
    field: Field,
    text: String,
    category: SlopCategory,
}

/// One line of the flags file.
#[derive(Deserialize, Serialize)]
struct SlopFlag {
    id: String,
    note: String,
    #[serde(default)]
    spans: Vec<SlopSpan>,
}

/// The body of `POST /flags`: the complete new state of one example's flag.
#[derive(Deserialize)]
struct FlagChange {
    id: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    spans: Vec<SlopSpan>,
    /// `true` sets or replaces the flag, `false` removes it.
    flagged: bool,
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServeError::Io { action, source } => write!(f, "{action}: {source}"),
            ServeError::Json(error) => write!(f, "invalid flag JSON: {error}"),
            ServeError::MissingTrainingFile(path) => write!(
                f,
                "{} not found; build it first with `cargo run --release -p thor-hammer-trainer`",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ServeError {}

impl From<serde_json::Error> for ServeError {
    fn from(error: serde_json::Error) -> Self {
        ServeError::Json(error)
    }
}

impl Response {
    /// A response with a body.
    fn ok(content_type: &'static str, body: Vec<u8>) -> Self {
        Response {
            status: "200 OK",
            content_type,
            body,
        }
    }

    /// A plain-text error response.
    fn error(status: &'static str, message: &str) -> Self {
        Response {
            status,
            content_type: "text/plain; charset=utf-8",
            body: message.as_bytes().to_vec(),
        }
    }
}

/// A converter for `map_err` that names what the server was doing.
fn io_error(action: &str) -> impl FnOnce(io::Error) -> ServeError {
    let action = action.to_string();
    move |source| ServeError::Io { action, source }
}

fn main() -> ExitCode {
    match serve_forever() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("review: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Reads the arguments, binds the port, and answers requests one at a time.
fn serve_forever() -> Result<(), ServeError> {
    let mut arguments = std::env::args().skip(1);
    let training = PathBuf::from(
        arguments
            .next()
            .unwrap_or_else(|| DEFAULT_TRAINING_FILE.to_string()),
    );
    let port = arguments
        .next()
        .unwrap_or_else(|| DEFAULT_PORT.to_string());
    let flags = PathBuf::from(
        arguments
            .next()
            .unwrap_or_else(|| DEFAULT_FLAGS_FILE.to_string()),
    );
    if !training.is_file() {
        return Err(ServeError::MissingTrainingFile(training));
    }
    let removed_as_slop = training.with_file_name("slop.jsonl");
    let files = Files {
        training,
        removed_as_slop,
        flags,
    };

    let address = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&address).map_err(io_error(&format!("binding {address}")))?;
    println!("Reviewing {} at http://{address}", files.training.display());
    println!("Slop flags are saved to {}", files.flags.display());

    for connection in listener.incoming() {
        let request_result = connection
            .map_err(io_error("accepting a connection"))
            .and_then(|stream| answer(stream, &files));
        if let Err(error) = request_result {
            eprintln!("review: {error}");
        }
    }
    Ok(())
}

/// Reads one request and writes the matching response.
fn answer(mut stream: TcpStream, files: &Files) -> Result<(), ServeError> {
    let request = read_request(&stream)?;
    let response = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => Response::ok("text/html; charset=utf-8", REVIEW_PAGE.as_bytes().to_vec()),
        ("GET", "/train.jsonl") => {
            let training_data =
                fs::read(&files.training).map_err(io_error("reading the training file"))?;
            Response::ok("application/x-ndjson; charset=utf-8", training_data)
        }
        ("GET", "/slop.jsonl") => Response::ok(
            "application/x-ndjson; charset=utf-8",
            read_optional_file(&files.removed_as_slop)?,
        ),
        ("GET", "/flags") => Response::ok(
            "application/x-ndjson; charset=utf-8",
            read_optional_file(&files.flags)?,
        ),
        ("POST", "/flags") => change_flag(&request, &files.flags),
        (method, path) => {
            eprintln!("review: no route for {method} {path}");
            Response::error("404 Not Found", "not found")
        }
    };
    write_response(&mut stream, &response)
}

/// Applies one `POST /flags` request and reports the outcome.
fn change_flag(request: &Request, flags_file: &Path) -> Response {
    if !request
        .content_type
        .starts_with("application/json")
    {
        return Response::error(
            "415 Unsupported Media Type",
            "send Content-Type: application/json",
        );
    }
    let outcome = serde_json::from_slice::<FlagChange>(&request.body)
        .map_err(ServeError::from)
        .and_then(|change| save_flag_change(flags_file, change));
    match outcome {
        Ok(()) => Response::ok("text/plain; charset=utf-8", b"saved".to_vec()),
        Err(error) => {
            eprintln!("review: {error}");
            Response::error("400 Bad Request", &error.to_string())
        }
    }
}

/// The file as bytes; empty when it does not exist yet.
fn read_optional_file(path: &Path) -> Result<Vec<u8>, ServeError> {
    match fs::read(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(io_error(&format!("reading {}", path.display()))(error)),
    }
}

/// Removes any existing flag for the id, adds the new one if `flagged`, and
/// rewrites the whole file.
fn save_flag_change(flags_file: &Path, change: FlagChange) -> Result<(), ServeError> {
    let current = String::from_utf8_lossy(&read_optional_file(flags_file)?).into_owned();
    let mut flags: Vec<SlopFlag> = Vec::new();
    for line in current
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let flag: SlopFlag = serde_json::from_str(line)?;
        if flag.id != change.id {
            flags.push(flag);
        }
    }
    if change.flagged {
        flags.push(SlopFlag {
            id: change.id,
            note: change.note.trim().to_string(),
            spans: change.spans,
        });
    }

    let mut flags_jsonl = String::new();
    for flag in &flags {
        flags_jsonl.push_str(&serde_json::to_string(flag)?);
        flags_jsonl.push('\n');
    }
    if let Some(labels_directory) = flags_file.parent() {
        fs::create_dir_all(labels_directory).map_err(io_error("creating the labels folder"))?;
    }
    fs::write(flags_file, flags_jsonl).map_err(io_error("writing the flags file"))
}

/// Reads the request line, the headers and, when `Content-Length` is set, the body.
fn read_request(stream: &TcpStream) -> Result<Request, ServeError> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(io_error("reading the request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .unwrap_or("")
        .to_string();
    let target = request_parts.next().unwrap_or("/");
    let path = target
        .split('?')
        .next()
        .unwrap_or(target)
        .to_string();

    let mut content_length = 0;
    let mut content_type = String::new();
    let mut header_line = String::new();
    while reader
        .read_line(&mut header_line)
        .map_err(io_error("reading headers"))?
        > 0
    {
        let Some((name, value)) = header_line.trim().split_once(':') else {
            break;
        };
        match name
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "content-length" => content_length = value.trim().parse().unwrap_or(0),
            "content-type" => content_type = value.trim().to_ascii_lowercase(),
            _ => {}
        }
        header_line.clear();
    }

    let mut body = vec![0; content_length.min(MAX_BODY_BYTES)];
    reader
        .read_exact(&mut body)
        .map_err(io_error("reading the request body"))?;
    Ok(Request {
        method,
        path,
        content_type,
        body,
    })
}

/// Writes the status line, headers and body, then lets the connection close.
fn write_response(stream: &mut TcpStream, response: &Response) -> Result<(), ServeError> {
    let headers = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .map_err(io_error("writing headers"))?;
    stream
        .write_all(&response.body)
        .map_err(io_error("writing the body"))
}
