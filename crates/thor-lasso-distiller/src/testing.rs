//! A model server for tests that answers each request with the next reply of a script.

use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex, PoisonError},
    thread,
};

use crate::{client::Client, error::DistillError};

/// The size of the buffer a request is read into; requests in tests are smaller.
const REQUEST_BUFFER: usize = 64 * 1024;

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
                let mut buffer = vec![0_u8; REQUEST_BUFFER];
                let read = stream.read(&mut buffer).unwrap_or_default();
                seen.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(String::from_utf8_lossy(&buffer[..read]).into_owned());
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
