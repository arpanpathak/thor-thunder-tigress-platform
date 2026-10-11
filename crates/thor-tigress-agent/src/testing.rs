//! A fake upstream server for tests: answers each connection with the next
//! canned response and records the raw requests it received.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};

use axum::http::StatusCode;

use crate::{
    web::{
        address::Url,
        http::{Fetched, Web},
    },
    error::{AgentError, Outcome},
    server::response::Client,
};

/// A server on `127.0.0.1` at a free port, serving canned responses in order.
pub struct FakeServer {
    address: String,
    serving: thread::JoinHandle<Vec<Outcome<String>>>,
}

impl FakeServer {
    /// Starts a server that accepts one connection per response in `responses`.
    pub fn start(responses: Vec<String>) -> Outcome<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let serving = thread::spawn(move || {
            responses
                .iter()
                .map(|response| serve_one(&listener, response))
                .collect()
        });

        Ok(FakeServer { address, serving })
    }

    /// The `host:port` to call.
    pub fn address(&self) -> String {
        self.address.clone()
    }

    /// Waits until every response is sent, and returns the requests received.
    pub fn requests(self) -> Outcome<Vec<String>> {
        self.serving
            .join()
            .map_err(|_| AgentError::Upstream("the fake server panicked".to_string()))?
            .into_iter()
            .collect()
    }
}

/// Accepts one connection, reads its request and answers with `response`.
fn serve_one(listener: &TcpListener, response: &str) -> Outcome<String> {
    let (mut stream, _) = listener.accept()?;
    let request = read_raw_request(&stream)?;

    stream.write_all(response.as_bytes())?;

    Ok(request)
}

/// Reads one request as text: headers, then as many body bytes as
/// `Content-Length` says.
fn read_raw_request(stream: &TcpStream) -> Outcome<String> {
    let mut reader = BufReader::new(stream);
    let mut raw = String::new();
    let mut length = 0;

    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        raw.push_str(&line);

        if line.trim().is_empty() {
            break;
        }

        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }

    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    raw.push_str(&String::from_utf8_lossy(&body));

    Ok(raw)
}

/// An `HTTP/1.1 200` response with a JSON body.
pub fn json_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// An `HTTP/1.1 200` event stream carrying `events`, then `[DONE]`.
pub fn event_stream(events: &[&str]) -> String {
    let mut response = String::from("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");

    for event in events {
        response.push_str("data: ");
        response.push_str(event);
        response.push_str("\n\n");
    }

    response.push_str("data: [DONE]\n\n");

    response
}

/// A writer whose every write fails, as if the client had gone.
pub struct Gone;

impl Write for Gone {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
}

impl Client for Gone {
    fn begin(&mut self, _: StatusCode, _: &str) -> Outcome {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into())
    }
}

/// A client kept in memory: the answer's head as text, then its body.
impl Client for Vec<u8> {
    fn begin(&mut self, status: StatusCode, content_type: &str) -> Outcome {
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n\r\n");

        self.extend_from_slice(head.as_bytes());

        Ok(())
    }
}

/// A stand-in for the real web: canned pages, keyed by address.
#[derive(Debug, Default)]
pub struct FakeWeb {
    pages: std::collections::HashMap<String, Fetched>,
}

impl FakeWeb {
    /// A fake holding `pages`, each keyed by its address text.
    #[must_use]
    pub fn new(pages: Vec<(&str, Fetched)>) -> Self {
        let mut fake = FakeWeb::default();

        for (url, page) in pages {
            fake.pages.insert((*url).to_string(), page);
        }

        fake
    }
}

impl Web for FakeWeb {
    fn get(&self, url: &Url) -> Outcome<Fetched> {
        self.pages
            .get(&url.as_string())
            .cloned()
            .ok_or_else(|| AgentError::Fetch(format!("{}: not in the fake", url.as_string())))
    }
}
