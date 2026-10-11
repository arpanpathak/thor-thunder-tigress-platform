//! `thor-tigress-agent`: the front server of the Thor Tigress Cub.
//!
//! It serves the chat page, checks the access key, passes model calls to the
//! server that has the model they name (llama-server, or a TensorRT Edge-LLM
//! engine), and, when the page turns web search on, lets the model call
//! `web_search` backed by SearXNG.
//! `/v1/chat/completions` is what OpenAI-compatible coding tools call and
//! `/v1/messages` what Claude Code calls. HTTP is axum on tokio; the handlers
//! that call an engine are blocking code on tokio's blocking pool.
//!
//! | Module | Does |
//! |---|---|
//! | [`config`] | the command line, the key file, the key check |
//! | [`server`] | the axum router, and a blocking handler's answer streamed as the response |
//! | [`chat`] | a chat request: tool rounds, web search, and the events sent to the page |
//! | [`engines`] | calling llama-server or an engine: the client, the model list, Anthropic requests |
//! | [`web`] | search, the source ledger, and reading a cited page and its links |

#![forbid(unsafe_code)]

mod chat;
mod config;
mod engines;
mod error;
mod paths;
mod server;
#[cfg(test)]
mod testing;
mod web;

use std::{process::ExitCode, sync::Arc};

use tokio::{
    net::TcpListener,
    signal::unix::{SignalKind, signal},
};

use crate::{
    config::{Config, Engine},
    error::Outcome,
};

/// Runs until the arguments, the key file or the listening address fail, or
/// the process is told to stop.
#[tokio::main]
async fn main() -> ExitCode {
    let outcome = match Config::from_args(std::env::args().skip(1)) {
        Ok(config) => serve(config).await,
        Err(error) => Err(error),
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("thor-tigress-agent: {error}");

            ExitCode::FAILURE
        }
    }
}

/// Binds the listening address and answers requests until the process is
/// told to stop (`SIGTERM`), which ends it at once, as before, but cleanly.
async fn serve(config: Config) -> Outcome {
    let listener = TcpListener::bind(&config.listen).await?;
    let mut terminate = signal(SignalKind::terminate())?;

    let access = if config.key.is_some() {
        "required"
    } else {
        "off"
    };

    let models: Vec<&str> = config.upstreams.engines().map(Engine::label).collect();

    eprintln!(
        "listening on {}, models from {}, search {}, access key {}, keyring {}",
        config.listen,
        models.join(", "),
        config.upstreams.search.address(),
        access,
        if config.people.is_some() { "on" } else { "off" }
    );

    tokio::spawn(axum::serve(listener, server::routes::router(Arc::new(config))).into_future());
    terminate.recv().await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

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

    async fn exchange(address: &str, request: &str) -> Outcome<String> {
        let mut stream = TcpStream::connect(address).await?;
        let mut reply = String::new();

        stream.write_all(request.as_bytes()).await?;
        stream.read_to_string(&mut reply).await?;

        Ok(reply)
    }

    #[tokio::test]
    async fn serves_over_a_real_connection_and_refuses_a_taken_address() -> Outcome {
        let free = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = free.local_addr()?.to_string();

        drop(free);

        let key_file = std::env::temp_dir().join(format!("thor-agent-key-{}", std::process::id()));

        std::fs::write(&key_file, "k\n")?;

        let key_path = key_file.display().to_string();
        let keyed = ["--listen", &address, "--key-file", &key_path];
        let config = Config::from_args(arguments(&keyed))?;
        let serving = tokio::spawn(serve(config));

        tokio::time::sleep(Duration::from_millis(200)).await;

        let health = "GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
        let chat = "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer k\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{x";
        let healthy = exchange(&address, health).await?;
        let broken = exchange(&address, chat).await?;
        let busy = serve(Config::from_args(arguments(&["--listen", &address]))?).await;

        serving.abort();
        std::fs::remove_file(&key_file)?;

        assert!(healthy.starts_with("HTTP/1.1 200 OK"), "{healthy}");
        assert!(healthy.ends_with(r#"{"status":"ok"}"#), "{healthy}");
        assert!(broken.starts_with("HTTP/1.1 400 Bad Request"), "{broken}");
        assert!(busy.is_err());

        Ok(())
    }
}
