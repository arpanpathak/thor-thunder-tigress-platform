//! `thor-tigress-telemetry`: ships the model servers' traffic to OpenObserve.
//!
//! Every few seconds it reads the lines `thor-tigress-agent` added to its
//! request log and sends them to the `requests` stream, then checks that the
//! agent and each engine answer and sends that, with the machine's memory and
//! load, to the `health` stream. OpenObserve stores both and draws the
//! dashboards. Requests that could not be sent are kept and sent next round.
//!
//! | Module | Does |
//! |---|---|
//! | [`cli`] | the command line |
//! | [`records`] | reads the request log as it grows |
//! | [`probes`] | server health, memory and load |
//! | [`ship`] | sends events to OpenObserve |

#![forbid(unsafe_code)]

mod cli;
mod error;
mod probes;
mod records;
mod ship;
#[cfg(test)]
mod testing;

use std::{
    process::ExitCode,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use crate::{
    cli::{Options, Target},
    error::Outcome,
    records::{Record, Tail},
    ship::OpenObserve,
};

/// An event with the time OpenObserve files it under.
#[derive(Debug, Serialize)]
struct Stamped<T> {
    #[serde(rename = "_timestamp")]
    timestamp_us: u64,
    kind: &'static str,
    #[serde(flatten)]
    body: T,
}

/// The log as read so far, and the requests not yet accepted by OpenObserve.
struct Shipper {
    sink: OpenObserve,
    tail: Tail,
    pending: Vec<Record>,
    targets: Vec<Target>,
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("thor-tigress-telemetry: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Reads the options and the password, then ships a round every `--every`
/// seconds, `--rounds` times or until stopped.
fn run(arguments: impl IntoIterator<Item = String>) -> anyhow::Result<()> {
    let options = Options::parse(arguments)?;
    let password = std::fs::read_to_string(&options.password_file)?;
    let mut shipper = Shipper {
        sink: OpenObserve::new(
            &options.openobserve,
            &options.org,
            &options.user,
            password.trim(),
        ),
        tail: Tail::new(options.log.clone()),
        pending: Vec::new(),
        targets: options.targets.clone(),
    };

    let mut shipped = 0;

    loop {
        report(shipper.round());
        shipped += 1;

        if options.rounds == Some(shipped) {
            return Ok(());
        }

        std::thread::sleep(Duration::from_secs(options.every));
    }
}

impl Shipper {
    /// Sends the new requests, then the health of every server and the machine.
    fn round(&mut self) -> Outcome {
        self.pending.extend(self.tail.read_new()?);
        self.sink.send("requests", &self.pending)?;
        self.pending.clear();

        let now = now_us();
        let servers = probes::check_all(&self.targets);
        let machine = probes::machine();
        let mut health: Vec<Stamped<serde_json::Value>> = stamped(now, "server", servers)?;

        health.extend(stamped(now, "machine", vec![machine])?);

        self.sink.send("health", &health)
    }
}

/// `items` as events of `kind` at `now`.
fn stamped<T: Serialize>(
    now: u64,
    kind: &'static str,
    items: Vec<T>,
) -> Outcome<Vec<Stamped<serde_json::Value>>> {
    items
        .into_iter()
        .map(|item| {
            let body = serde_json::to_value(item)?;

            Ok(Stamped {
                timestamp_us: now,
                kind,
                body,
            })
        })
        .collect()
}

/// Says what went wrong in a round; the next round tries again.
fn report(round: Outcome) {
    if let Err(error) = round {
        eprintln!("thor-tigress-telemetry: {error}");
    }
}

fn now_us() -> u64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    u64::try_from(since.as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;

    use super::*;
    use crate::testing::fake_openobserve;

    fn arguments(
        log: &std::path::Path,
        password: &std::path::Path,
        openobserve: &str,
    ) -> Vec<String> {
        [
            "--log",
            &log.display().to_string(),
            "--password-file",
            &password.display().to_string(),
            "--openobserve",
            openobserve,
            "--agent",
            "127.0.0.1:1",
            "--rounds",
            "1",
        ]
        .iter()
        .map(ToString::to_string)
        .collect()
    }

    #[test]
    fn one_round_sends_the_requests_then_the_health() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let log = folder.path().join("requests.jsonl");
        let password = folder.path().join("password");
        let (address, received) = fake_openobserve(vec!["200 OK", "200 OK"])?;

        fs::write(&log, "{\"at_ms\":5,\"who\":\"ada\",\"status\":200}\n")?;
        fs::write(&password, "p4ss\n")?;
        run(arguments(&log, &password, &address))?;

        let requests = received.recv()?;
        let health = received.recv()?;

        assert!(requests.starts_with("POST /api/default/requests/_json "));
        assert!(requests.contains(r#""_timestamp":5000"#));
        assert!(health.starts_with("POST /api/default/health/_json "));
        assert!(health.contains(r#""kind":"server""#));
        assert!(health.contains(r#""name":"thor-tigress-agent""#));
        assert!(health.contains(r#""kind":"machine""#));

        Ok(())
    }

    #[test]
    fn requests_that_were_not_accepted_are_sent_next_round() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let log = folder.path().join("requests.jsonl");
        let (address, received) =
            fake_openobserve(vec!["503 Service Unavailable", "200 OK", "200 OK"])?;
        let mut shipper = Shipper {
            sink: OpenObserve::new(&address, "default", "u", "p"),
            tail: Tail::new(log.clone()),
            pending: Vec::new(),
            targets: Vec::new(),
        };

        fs::write(&log, "{\"at_ms\":5,\"who\":\"ada\"}\n")?;

        let refused = shipper.round();
        let pending = shipper.pending.len();
        shipper.round()?;

        assert!(refused.is_err());
        assert_eq!(pending, 1);
        assert!(shipper.pending.is_empty());
        assert!(received.recv()?.contains("ada"));
        assert!(received.recv()?.contains("ada"));

        Ok(())
    }

    #[test]
    fn a_failed_round_is_reported_and_a_missing_password_ends_the_run() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let log = folder.path().join("requests.jsonl");
        let password = folder.path().join("password");
        let closed = format!(
            "http://{}",
            std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?
        );

        let without_password = run(arguments(&log, &password, &closed));

        fs::write(&password, "p4ss")?;
        fs::write(&log, "{}\n")?;

        let mut twice = arguments(&log, &password, &closed);
        twice.extend(["--rounds", "2", "--every", "1"].map(String::from));

        assert!(without_password.is_err());
        assert!(run(twice).is_ok());

        Ok(())
    }
}
