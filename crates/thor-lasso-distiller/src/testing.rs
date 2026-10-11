//! A model server for tests that answers each request with the next reply of a script.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, PoisonError},
    thread,
};

use crate::{client::Client, error::DistillError};

/// What the server saw of each connection: the request, or why it failed.
type Seen = Arc<Mutex<Vec<Result<String, DistillError>>>>;

/// A server on a free local port that answers the n-th connection with the
/// n-th raw response and keeps every request.
pub struct FakeServer {
    /// The `host:port` to call.
    pub address: String,
    seen: Seen,
}

impl FakeServer {
    /// Starts a server that sends `responses`, in order, then closes connections.
    ///
    /// # Errors
    ///
    /// `DistillError::Io` when no local port can be bound.
    pub fn start(responses: Vec<String>) -> Result<Self, DistillError> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(DistillError::io("listener"))?;
        let address = listener
            .local_addr()
            .map_err(DistillError::io("listener"))?
            .to_string();
        let seen = Seen::default();
        let record = Arc::clone(&seen);

        thread::spawn(move || {
            for response in responses {
                let served = serve_one(&listener, &response);

                record
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(served);
            }
        });

        Ok(Self { address, seen })
    }

    /// How many requests the server has answered.
    #[must_use]
    pub fn request_count(&self) -> usize {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|served| served.is_ok())
            .count()
    }
}

/// A [`FakeServer`] that replies as an OpenAI-style chat completion, with a
/// client pointed at it.
pub struct FakeModel {
    /// A client pointed at the server.
    pub client: Client,
    server: FakeServer,
}

impl FakeModel {
    /// Starts a server that answers with `replies`, in order, then closes connections.
    ///
    /// # Errors
    ///
    /// `DistillError::Io` when no local port can be bound.
    pub fn start(replies: &[&str]) -> Result<Self, DistillError> {
        let server = FakeServer::start(replies.iter().map(|reply| completion(reply)).collect())?;
        let client = Client {
            address: server.address.clone(),
            model: "teacher".to_string(),
            key: None,
        };

        Ok(Self { client, server })
    }

    /// How many requests the server has answered.
    #[must_use]
    pub fn request_count(&self) -> usize {
        self.server.request_count()
    }
}

/// `reply` as the HTTP response of a chat completion.
fn completion(reply: &str) -> String {
    let body =
        serde_json::json!({ "choices": [{ "message": { "role": "assistant", "content": reply } }] })
            .to_string();

    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Accepts one connection, reads its request and answers with `response`.
fn serve_one(listener: &TcpListener, response: &str) -> Result<String, DistillError> {
    let (mut stream, _) = listener.accept().map_err(DistillError::io("connection"))?;
    let request = read_request(&stream)?;

    stream
        .write_all(response.as_bytes())
        .map_err(DistillError::io("response"))?;

    Ok(request)
}

/// Reads one whole request as text: the headers, then as many body bytes as
/// `Content-Length` says. Answering before the body has arrived would reset
/// the connection under a client that sends the two separately.
fn read_request(stream: &TcpStream) -> Result<String, DistillError> {
    let mut reader = BufReader::new(stream);
    let mut raw = String::new();
    let mut length = 0;

    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(DistillError::io("request"))?;
        raw.push_str(&line);

        if line.trim().is_empty() {
            break;
        }

        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }

    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(DistillError::io("request"))?;
    raw.push_str(&String::from_utf8_lossy(&body));

    Ok(raw)
}
