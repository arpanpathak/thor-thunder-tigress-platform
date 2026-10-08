//! Just enough HTTP to call an OpenAI-compatible `/v1/chat/completions`
//! endpoint on localhost, which is what `trtllm-serve` exposes. Talking HTTP
//! keeps the engine swappable without touching this crate.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

use serde_json::{Value, json};

use crate::error::DistillError;

/// How long one request may take before it is given up on.
const TIMEOUT: Duration = Duration::from_secs(300);

/// One chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// `system`, `user` or `assistant`.
    pub role: &'static str,
    /// The text.
    pub content: String,
}

/// A server and the model name it serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    /// `host:port`, such as `127.0.0.1:8000`.
    pub address: String,
    /// The model name the server expects in requests.
    pub model: String,
    /// The access key, sent as `Authorization: Bearer <key>` when set.
    pub key: Option<String>,
}

impl Client {
    /// Sends `messages` and returns the reply text of the first choice.
    /// Thinking is switched off (`enable_thinking: false`, which servers that
    /// do not know it ignore): a hybrid reasoning model would otherwise spend
    /// the whole token budget reasoning and return no question at all.
    pub fn complete(
        &self,
        messages: &[Message],
        max_tokens: u32,
        temperature: f32,
    ) -> Result<String, DistillError> {
        let body = json!({
            "model": self.model,
            "messages": messages
                .iter()
                .map(|message| json!({ "role": message.role, "content": message.content }))
                .collect::<Vec<Value>>(),
            "max_tokens": max_tokens,
            "temperature": temperature,
            "chat_template_kwargs": { "enable_thinking": false },
        })
        .to_string();
        let response = self.post("/v1/chat/completions", &body)?;
        let parsed: Value = serde_json::from_str(&response)
            .map_err(|error| DistillError::Server(format!("reply is not JSON: {error}")))?;
        parsed
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| DistillError::Server(format!("reply has no message: {response}")))
    }

    fn post(&self, path: &str, body: &str) -> Result<String, DistillError> {
        let server =
            |error: std::io::Error| DistillError::Server(format!("{}: {error}", self.address));
        let mut stream = TcpStream::connect(&self.address).map_err(server)?;
        stream.set_read_timeout(Some(TIMEOUT)).map_err(server)?;
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {}\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.address,
            self.key.as_ref().map_or(String::new(), |key| format!(
                "Authorization: Bearer {key}\r\n"
            )),
            body.len()
        );
        stream.write_all(request.as_bytes()).map_err(server)?;
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status).map_err(server)?;
        let mut chunked = false;

        loop {
            let mut header = String::new();
            reader.read_line(&mut header).map_err(server)?;

            if header.trim().is_empty() {
                break;
            }

            let lowered = header.to_ascii_lowercase();
            chunked |= lowered.starts_with("transfer-encoding:") && lowered.contains("chunked");
        }
        let mut raw = Vec::new();
        reader.read_to_end(&mut raw).map_err(server)?;
        let text = String::from_utf8_lossy(&raw).into_owned();
        let body = if chunked { dechunk(&text) } else { text };

        match status.split_whitespace().nth(1) {
            Some("200") => Ok(body),
            _ => Err(DistillError::Server(format!(
                "{} {}",
                status.trim(),
                body.trim()
            ))),
        }
    }
}

/// The body of a `Transfer-Encoding: chunked` response.
fn dechunk(text: &str) -> String {
    let mut body = String::new();
    let mut rest = text;

    while let Some((size_line, after)) = rest.split_once("\r\n") {
        let size = usize::from_str_radix(size_line.trim(), 16).unwrap_or(0);

        if size == 0 {
            break;
        }

        body.push_str(after.get(..size).unwrap_or(after));
        rest = after.get(size..).unwrap_or("").trim_start_matches("\r\n");
    }
    body
}

#[cfg(test)]
mod tests {
    use std::{net::TcpListener, thread};

    use super::*;

    fn serve_once(reply: String) -> Result<String, DistillError> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(DistillError::io("listener"))?;
        let address = listener
            .local_addr()
            .map_err(DistillError::io("listener"))?
            .to_string();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        Ok(address)
    }

    #[test]
    fn reads_the_first_choice_of_a_reply() -> Result<(), DistillError> {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"How do I share a Vec between threads?"}}]}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let client = Client {
            address: serve_once(reply)?,
            model: "teacher".to_string(),
            key: None,
        };
        let message = Message {
            role: "user",
            content: "hi".to_string(),
        };
        assert_eq!(
            client.complete(&[message], 32, 0.7)?,
            "How do I share a Vec between threads?"
        );
        Ok(())
    }

    #[test]
    fn reports_a_server_error() -> Result<(), DistillError> {
        let client = Client {
            address: serve_once(
                "HTTP/1.1 500 Internal Server Error\r\n\r\nengine not loaded".to_string(),
            )?,
            model: "teacher".to_string(),
            key: None,
        };
        let outcome = client.complete(&[], 32, 0.7);
        assert!(
            matches!(outcome, Err(DistillError::Server(message)) if message.contains("engine not loaded"))
        );
        Ok(())
    }

    #[test]
    fn reads_a_chunked_reply() -> Result<(), DistillError> {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"chunked"}}]}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
            body.len()
        );
        let client = Client {
            address: serve_once(reply)?,
            model: "teacher".to_string(),
            key: Some("k".to_string()),
        };
        assert_eq!(client.complete(&[], 8, 0.0)?, "chunked");
        Ok(())
    }

    #[test]
    fn a_reply_without_json_or_a_message_and_a_closed_port_are_server_errors()
    -> Result<(), DistillError> {
        let client = |address: String| Client {
            address,
            model: "teacher".to_string(),
            key: None,
        };
        let not_json = client(serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nnope".to_string(),
        )?);
        assert!(
            matches!(not_json.complete(&[], 8, 0.0), Err(DistillError::Server(message)) if message.starts_with("reply is not JSON"))
        );
        let empty = client(serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_string(),
        )?);
        assert!(
            matches!(empty.complete(&[], 8, 0.0), Err(DistillError::Server(message)) if message.starts_with("reply has no message"))
        );
        let closed =
            std::net::TcpListener::bind("127.0.0.1:0").map_err(DistillError::io("listener"))?;
        let address = closed
            .local_addr()
            .map_err(DistillError::io("listener"))?
            .to_string();
        drop(closed);
        assert!(matches!(
            client(address).complete(&[], 8, 0.0),
            Err(DistillError::Server(_))
        ));
        Ok(())
    }

    #[test]
    fn joins_chunked_bodies() {
        assert_eq!(
            dechunk("5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"),
            "hello world"
        );
    }
}
