//! `thor-tigress-agent`: the front server of the Thor chat.
//!
//! ```text
//! thor-tigress-agent [--listen 127.0.0.1:8080] [--model 127.0.0.1:8079]
//!                    [--search 127.0.0.1:8888] [--web DIR] [--key-file FILE]
//! ```
//!
//! It serves the chat page, checks the access key, passes model calls to
//! `llama-server`, and, when the page turns web search on, lets the model call
//! `web_search` backed by SearXNG. `/v1/messages` (Anthropic's API, used by
//! Claude Code) goes to `llama-server` unchanged. One thread per connection.

mod agent;
mod error;
mod http;
mod search;

use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
    thread,
};

use crate::{agent::Upstreams, error::AgentError};

/// The settings of a running server.
struct Config {
    listen: String,
    web: PathBuf,
    key: Option<String>,
    upstreams: Upstreams,
}

fn main() -> ExitCode {
    match config(std::env::args().skip(1).collect()).and_then(serve) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("thor-tigress-agent: {error}");
            ExitCode::FAILURE
        }
    }
}

fn config(arguments: Vec<String>) -> Result<Config, AgentError> {
    let mut listen = "127.0.0.1:8080".to_string();
    let mut model = "127.0.0.1:8079".to_string();
    let mut search = "127.0.0.1:8888".to_string();
    let mut web = PathBuf::from(".");
    let mut key_file = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".config/thor-chat/api-key"))
        .unwrap_or_default();
    let mut rest = arguments.into_iter();
    while let Some(flag) = rest.next() {
        let value = rest
            .next()
            .ok_or_else(|| AgentError::Config(format!("{flag} needs a value")))?;
        match flag.as_str() {
            "--listen" => listen = value,
            "--model" => model = value,
            "--search" => search = value,
            "--web" => web = PathBuf::from(value),
            "--key-file" => key_file = PathBuf::from(value),
            other => return Err(AgentError::Config(format!("unknown option {other}"))),
        }
    }
    let key = fs::read_to_string(&key_file)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    let authorization = key.as_ref().map(|key| format!("Bearer {key}"));
    Ok(Config {
        listen,
        web,
        key,
        upstreams: Upstreams {
            model,
            search,
            authorization,
        },
    })
}

fn serve(config: Config) -> Result<(), AgentError> {
    let listener = TcpListener::bind(&config.listen)?;
    eprintln!(
        "listening on {}, model {}, search {}, access key {}",
        config.listen,
        config.upstreams.model,
        config.upstreams.search,
        if config.key.is_some() { "required" } else { "off" }
    );
    let config = Arc::new(config);
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let config = Arc::clone(&config);
        thread::spawn(move || {
            if let Err(error) = handle(stream, &config) {
                eprintln!("request failed: {error}");
            }
        });
    }
    Ok(())
}

fn handle(mut stream: TcpStream, config: &Config) -> Result<(), AgentError> {
    let request = http::read_request(&stream)?;
    let needs_key = request.path.starts_with("/v1/");
    let key_ok = config.key.as_ref().is_none_or(|key| {
        request.authorization.as_deref() == Some(format!("Bearer {key}").as_str())
    });
    match (request.method.as_str(), request.path.as_str()) {
        (_, _) if needs_key && !key_ok => http::respond(
            &mut stream,
            "401 Unauthorized",
            "application/json",
            br#"{"error":{"code":401,"message":"Invalid API Key","type":"authentication_error"}}"#,
        ),
        ("GET", "/" | "/index.html") => {
            let page = fs::read(config.web.join("index.html"))?;
            http::respond(&mut stream, "200 OK", "text/html; charset=utf-8", &page)
        }
        ("GET", "/health") => http::respond(&mut stream, "200 OK", "application/json", br#"{"status":"ok"}"#),
        ("GET", "/v1/models") => {
            let response = http::call(
                &config.upstreams.model,
                "GET",
                "/v1/models",
                config.upstreams.authorization.as_deref(),
                None,
            )?;
            let status = if response.status == 200 { "200 OK" } else { "502 Bad Gateway" };
            let body = response.text()?;
            http::respond(&mut stream, status, "application/json", body.as_bytes())
        }
        ("POST", "/v1/chat/completions") => agent::chat(&mut stream, &request.body, &config.upstreams),
        ("POST", path @ ("/v1/messages" | "/v1/messages/count_tokens")) => {
            let response = http::call(
                &config.upstreams.model,
                "POST",
                path,
                config.upstreams.authorization.as_deref(),
                Some(&request.body),
            )?;
            http::relay(&mut stream, response)
        }
        _ => http::respond(&mut stream, "404 Not Found", "text/plain", b"not found"),
    }
}
