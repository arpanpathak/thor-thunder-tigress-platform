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
mod request;
mod response;
mod routes;
mod search;
#[cfg(test)]
mod testing;
mod upstream;

use std::{net::TcpListener, process::ExitCode, sync::Arc, thread};

use crate::{config::Config, error::AgentError};

fn main() -> ExitCode {
    let outcome = Config::from_args(std::env::args().skip(1)).and_then(serve);
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("thor-tigress-agent: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Accepts connections forever, one thread each.
fn serve(config: Config) -> Result<(), AgentError> {
    let listener = TcpListener::bind(&config.listen)?;
    eprintln!(
        "listening on {}, model {}, search {}, access key {}",
        config.listen,
        config.upstreams.model.address(),
        config.upstreams.search.address(),
        if config.key.is_some() { "required" } else { "off" }
    );
    let config = Arc::new(config);
    for connection in listener.incoming() {
        let Ok(mut stream) = connection else {
            continue;
        };
        let config = Arc::clone(&config);
        thread::spawn(move || {
            let outcome = request::read_request(&stream).and_then(|request| routes::answer(&mut stream, &request, &config));
            if let Err(error) = outcome {
                eprintln!("request failed: {error}");
            }
        });
    }
    Ok(())
}
