//! `thor-tigress-agent`: the front server of the Thor Tigress Cub.
//!
//! It serves the chat page, checks the access key, passes model calls to the
//! TensorRT Edge-LLM engine serving the model they name, and, when the page
//! turns web search on, lets the model call `web_search` backed by SearXNG.
//! `/v1/chat/completions` is what OpenAI-compatible coding tools call and
//! `/v1/messages` what Claude Code calls. One thread per connection.
//!
//! | Module | Does |
//! |---|---|
//! | [`config`] | the command line, the key file, the key check |
//! | [`request`], [`response`] | reading requests, writing answers |
//! | [`routes`] | which request goes where |
//! | [`chat`], [`messages`], [`search`] | the model and search work |
//! | [`research`] | how a question becomes searches, and the source ledger |
//! | [`address`], [`fetch`], [`html`], [`http`] | reading a cited page and its links |
//! | [`tooltext`] | tool calls the model writes as text |
//! | [`body`] | a JSON request body and the model it names |
//! | [`models`] | the models the engines offer |
//! | [`upstream`] | calling an engine and SearXNG |

#![forbid(unsafe_code)]

mod address;
mod body;
mod chat;
mod config;
mod error;
mod fetch;
mod html;
mod http;
mod messages;
mod models;
mod paths;
mod request;
mod research;
mod response;
mod routes;
mod search;
#[cfg(test)]
mod testing;
mod tooltext;
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
    let access = if config.key.is_some() { "required" } else { "off" };
    let engines: Vec<&str> = config
        .upstreams
        .engines()
        .map(|engine| engine.model.as_str())
        .collect();

    eprintln!(
        "listening on {}, engines {}, search {}, access key {}, keyring {}",
        config.listen,
        engines.join(", "),
        config.upstreams.search.address(),
        access,
        if config.people.is_some() { "on" } else { "off" }
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
    let outcome =
        request::read_request(stream).and_then(|request| routes::answer(stream, &request, config));
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

    /// `more`, behind an engine and a key file that isn't there.
    fn arguments(more: &[&str]) -> Vec<String> {
        let engine = ["--engine", "nemotron=127.0.0.1:1"];
        let key = ["--key-file", "/nonexistent"];

        engine
            .iter()
            .chain(&key)
            .chain(more)
            .map(ToString::to_string)
            .collect()
    }

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
        let config = Config::from_args(arguments(&[]))?;
        let request = b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\n\r\n{x".to_vec();
        let mut client = Vanishing {
            request: io::Cursor::new(request),
        };
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
        let config = Config::from_args(arguments(&["--listen", "127.0.0.1:0"]))?;
        thread::spawn(move || serve(config));
        thread::sleep(std::time::Duration::from_millis(200));
        let taken = TcpListener::bind("127.0.0.1:0")?;
        let address = taken.local_addr()?.to_string();
        let busy = Config::from_args(arguments(&["--listen", &address]))?;
        assert!(serve(busy).is_err());
        Ok(())
    }

    #[test]
    fn answers_connections_and_reports_bad_requests() -> Outcome {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let config = Arc::new(Config::from_args(arguments(&[]))?);
        let exchanged = thread::scope(|scope| {
            scope.spawn(|| accept(listener.incoming().take(2), &config));
            let health = exchange(&address, "GET /health HTTP/1.1\r\nHost: x\r\n\r\n")?;
            let raw = "POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\n\r\n{x";
            let broken = exchange(&address, raw)?;
            Ok::<_, crate::error::AgentError>((health, broken))
        });
        let (health, broken) = exchanged?;
        assert!(health.starts_with("HTTP/1.1 200 OK"), "{health}");
        assert!(broken.starts_with("HTTP/1.1 400 Bad Request"), "{broken}");
        Ok(())
    }
}
