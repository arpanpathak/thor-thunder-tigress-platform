//! Runs the agent as a program: the cases that end it (arguments it can't
//! read, an address it can't listen on), and every route of a running server.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
};

use anyhow::Context;

#[test]
fn exits_with_failure_and_a_reason() -> anyhow::Result<()> {
    let program = env!("CARGO_BIN_EXE_thor-tigress-agent");
    let unknown = Command::new(program).arg("--colour").output()?;

    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).starts_with("thor-tigress-agent: "));

    let taken = TcpListener::bind("127.0.0.1:0")?;
    let address = taken.local_addr()?.to_string();
    let busy = Command::new(program)
        .args(["--listen", &address, "--key-file", "/nonexistent"])
        .args(["--engine", "nemotron=127.0.0.1:1"])
        .output()?;

    assert!(!busy.status.success());
    assert!(String::from_utf8_lossy(&busy.stderr).starts_with("thor-tigress-agent: "));

    Ok(())
}

/// Sends `request` to the server at `address` and returns the status line of
/// its answer.
fn status_of(address: &str, request: &str) -> anyhow::Result<String> {
    let mut stream = TcpStream::connect(address)?;
    let mut reply = String::new();

    stream.write_all(request.as_bytes())?;
    stream.read_to_string(&mut reply)?;

    Ok(reply.lines().next().unwrap_or_default().to_string())
}

fn post(path: &str, body: &str) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

#[test]
fn answers_every_route_as_a_running_server() -> anyhow::Result<()> {
    let free = TcpListener::bind("127.0.0.1:0")?;
    let address = free.local_addr()?.to_string();

    drop(free);

    let mut server = Command::new(env!("CARGO_BIN_EXE_thor-tigress-agent"))
        .args(["--listen", &address, "--key-file", "/nonexistent"])
        .args(["--engine", "nemotron=127.0.0.1:1", "--web", "/nonexistent"])
        .stderr(Stdio::piped())
        .spawn()?;
    let log = server.stderr.take().context("no standard error")?;
    let mut logged = BufReader::new(log).lines().map_while(Result::ok);
    let listening = logged.any(|line| line.starts_with("listening on "));
    let question = r#"{"messages":[{"role":"user","content":"hi"}]}"#;
    let form = r#"{"name":"Ada","email":"ada@example.com"}"#;
    let get = |path: &str| format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    let requests = [
        get("/health"),
        get("/v1/models"),
        get("/"),
        get("/nowhere"),
        post("/v1/chat/completions", question),
        post("/v1/messages", question),
        post("/v1/messages/count_tokens", question),
        post("/request", form),
    ];
    let statuses = requests
        .iter()
        .map(|request| status_of(&address, request))
        .collect::<anyhow::Result<Vec<String>>>();

    let told = Command::new("kill")
        .args(["-TERM", &server.id().to_string()])
        .status()?;
    let stopped = server.wait()?;
    let failures = logged.filter(|line| line.starts_with("request failed: "));

    assert!(listening);
    assert!(told.success() && stopped.success());
    assert_eq!(failures.count(), 5);
    assert_eq!(
        statuses?,
        [
            "HTTP/1.1 200 OK",
            "HTTP/1.1 200 OK",
            "HTTP/1.1 500 Internal Server Error",
            "HTTP/1.1 404 Not Found",
            "HTTP/1.1 502 Bad Gateway",
            "HTTP/1.1 502 Bad Gateway",
            "HTTP/1.1 502 Bad Gateway",
            "HTTP/1.1 400 Bad Request",
        ]
    );

    Ok(())
}
