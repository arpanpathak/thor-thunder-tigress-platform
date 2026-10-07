//! `thor-tigress-agent`: the front server of the Thor Tigress Cub.
//!
//! It serves the chat page, checks the access key, passes model calls to
//! llama-server, and, when the page turns web search on, lets the model call
//! `web_search` backed by SearXNG. `/v1/messages` (Anthropic's API, used by
//! Claude Code) goes to llama-server with thinking off unless the request asks
//! for it. One thread per connection.
//!
//! | Module | Does |
//! |---|---|
//! | [`config`] | the command line, the key file, the key check |
//! | [`request`], [`response`] | reading requests, writing answers |
//! | [`routes`] | which request goes where |
//! | [`chat`], [`messages`], [`search`] | the model and search work |
//! | [`upstream`] | calling llama-server and SearXNG |

#![forbid(unsafe_code)]

mod chat;
mod config;
mod error;
mod messages;
mod models;
mod paths;
mod request;
mod response;
mod routes;
mod search;
#[cfg(test)]
mod testing;
mod upstream;

use std::{
    convert::Infallible,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    process::ExitCode,
    sync::Arc,
    thread,
};

use crate::{config::Config, error::Outcome};

/// Runs until the arguments, the key file or the listening address fail;
/// serving itself never ends.
fn main() -> ExitCode {
    let Err(error) = Config::from_args(std::env::args().skip(1)).and_then(serve);
    eprintln!("thor-tigress-agent: {error}");
    ExitCode::FAILURE
}

/// Accepts connections forever, one thread each.
fn serve(config: Config) -> Outcome<Infallible> {
    let listener = TcpListener::bind(&config.listen)?;
    eprintln!(
        "listening on {}, model {}, search {}, access key {}",
        config.listen,
        config.upstreams.model.address(),
        config.upstreams.search.address(),
        if config.key.is_some() { "required" } else { "off" }
    );
    let config = Arc::new(config);
    loop {
        accept(listener.incoming(), &config);
    }
}

/// Answers each connection on its own thread, until `connections` ends.
fn accept(connections: impl Iterator<Item = io::Result<TcpStream>>, config: &Arc<Config>) {
    for mut stream in connections.filter_map(Result::ok) {
        let config = Arc::clone(config);
        thread::spawn(move || handle(&mut stream, &config));
    }
}

/// A client connection: something to read the request from and write the answer to.
trait Connection: Read + Write {}

impl<T: Read + Write> Connection for T {}

/// Answers one connection; a failure is logged and, when the client is still
/// there, answered with its status.
fn handle(stream: &mut dyn Connection, config: &Config) {
    let outcome = request::read_request(stream).and_then(|request| routes::answer(stream, &request, config));
    let Err(error) = outcome else {
        return;
    };
    eprintln!("request failed: {error}");
    if let Err(unsent) = response::failure(stream, &error) {
        eprintln!("could not tell the client: {unsent}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client that sends `request` and is gone before the answer.
    struct Vanishing {
        request: io::Cursor<Vec<u8>>,
    }

    impl Read for Vanishing {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.request.read(buffer)
        }
    }

    impl Write for Vanishing {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            crate::testing::Gone.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            crate::testing::Gone.flush()
        }
    }

    #[test]
    fn a_client_gone_before_the_answer_is_only_logged() -> Outcome {
        let config = Config::from_args(["--key-file".to_string(), "/nonexistent".to_string()])?;
        let request = b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\n\r\n{x".to_vec();
        let mut client = Vanishing { request: io::Cursor::new(request) };
        handle(&mut client, &config);
        assert!(client.flush().is_err());
        Ok(())
    }

    fn exchange(address: &str, request: &str) -> Outcome<String> {
        let mut stream = TcpStream::connect(address)?;
        stream.write_all(request.as_bytes())?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply)?;
        Ok(reply)
    }

    #[test]
    fn serve_binds_and_announces() -> Outcome {
        let config = Config::from_args(["--listen", "127.0.0.1:0", "--key-file", "/nonexistent"].map(String::from))?;
        thread::spawn(move || serve(config));
        thread::sleep(std::time::Duration::from_millis(200));
        let taken = TcpListener::bind("127.0.0.1:0")?;
        let busy = Config::from_args(["--listen".to_string(), taken.local_addr()?.to_string(), "--key-file".to_string(), "/nonexistent".to_string()])?;
        assert!(serve(busy).is_err());
        Ok(())
    }

    #[test]
    fn answers_connections_and_reports_bad_requests() -> Outcome {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let config = Arc::new(Config::from_args(["--key-file".to_string(), "/nonexistent".to_string()])?);
        let exchanged = thread::scope(|scope| {
            scope.spawn(|| accept(listener.incoming().take(2), &config));
            let health = exchange(&address, "GET /health HTTP/1.1\r\nHost: x\r\n\r\n")?;
            let broken = exchange(&address, "POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\n\r\n{x")?;
            Ok::<_, crate::error::AgentError>((health, broken))
        });
        let (health, broken) = exchanged?;
        assert!(health.starts_with("HTTP/1.1 200 OK"), "{health}");
        assert!(broken.starts_with("HTTP/1.1 400 Bad Request"), "{broken}");
        Ok(())
    }
}
