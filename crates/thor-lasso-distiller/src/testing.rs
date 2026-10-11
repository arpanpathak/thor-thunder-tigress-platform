//! A model server for tests that answers each request with the next reply of a script.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, PoisonError},
    thread,
};

use crate::{client::Client, error::DistillError};

/// A server on a free local port that replies to the n-th request with the
/// n-th reply, as an OpenAI-style chat completion, and keeps every request body.
pub struct FakeModel {
    /// A client pointed at the server.
    pub client: Client,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeModel {
    /// Starts a server that answers with `replies`, in order, then closes connections.
    ///
    /// # Errors
    ///
    /// `DistillError::Io` when no local port can be bound.
    pub fn start(replies: &[&str]) -> Result<Self, DistillError> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(DistillError::io("listener"))?;
        let address = listener
            .local_addr()
            .map_err(DistillError::io("listener"))?
            .to_string();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let bodies: Vec<String> = replies
            .iter()
            .map(|reply| serde_json::json!({ "choices": [{ "message": { "role": "assistant", "content": reply } }] }).to_string())
            .collect();

        thread::spawn(move || {
            for (body, mut stream) in bodies
                .into_iter()
                .zip(listener.incoming().filter_map(Result::ok))
            {
                let request = read_request(&stream).unwrap_or_default();

                seen.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(request);

                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });

        let client = Client {
            address,
            model: "teacher".to_string(),
            key: None,
        };

        Ok(Self { client, requests })
    }

    /// How many requests the server has answered.
    #[must_use]
    pub fn request_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

/// Reads one whole request as text: the headers, then as many body bytes as
/// `Content-Length` says. Answering before the body has arrived would reset
/// the connection under a client that sends the two separately.
///
/// # Errors
///
/// `DistillError::Io` when the connection ends before the whole request arrived.
pub fn read_request(stream: &TcpStream) -> Result<String, DistillError> {
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
