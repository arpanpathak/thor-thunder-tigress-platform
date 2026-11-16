//! The systemd user services that run the engines, through `systemctl --user` and
//! `journalctl --user`, and the engines' own `/v1/models`.
//!
//! Both programs are found on the PATH, so tests put a fake `systemctl` first.

use std::{
    process::{Child, Command, Stdio},
    sync::LazyLock,
    thread,
    time::{Duration, Instant},
};

use regex::Regex;
use serde_json::Value;

use crate::{
    error::{Outcome, ThorError},
    web,
};

/// Lines in a service's log that say why it stopped: out of memory, an error, a crash.
static ERROR_LINE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)(out of memory|\berror\b|traceback|\boom|killed|failed|exception)").ok()
});

/// How often the wait checks the service.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// What `systemctl --user show` says about one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitState {
    /// The service's name, such as `thor-edge-llm`.
    pub name: String,
    /// `loaded` when its unit file exists, `not-found` when it does not.
    pub load_state: String,
    /// `active`, `activating`, `inactive` or `failed`.
    pub active_state: String,
    /// The memory its processes hold now, GPU buffers included, when systemd tracks it.
    pub memory: Option<u64>,
    /// Its `MemoryMax`, when it has one.
    pub memory_max: Option<u64>,
    /// How many times systemd has restarted it.
    pub restarts: u64,
    /// The memory of the Docker container the service started, which systemd does not
    /// count; `None` for services without one or when the container is not running.
    pub container: Option<u64>,
}

impl UnitState {
    /// Whether the service has failed, or systemd has restarted it, since it had
    /// `restarts_before` restarts.
    pub fn has_failed_since(&self, restarts_before: u64) -> bool {
        self.active_state == "failed" || self.restarts > restarts_before
    }

    /// The memory the service holds: its container's when it has one, else its own.
    pub fn held_memory(&self) -> Option<u64> {
        self.container.or(self.memory)
    }

    /// Whether the service, or a container it left behind, holds memory now.
    pub fn is_holding_memory(&self) -> bool {
        self.is_running() || self.container.is_some()
    }

    /// Whether the service is running or starting.
    pub fn is_running(&self) -> bool {
        matches!(
            self.active_state.as_str(),
            "active" | "activating" | "reloading"
        )
    }
}

/// How a start ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// The server lists the model on `/v1/models`.
    Ready,
    /// The service failed or was restarted by systemd.
    Failed,
    /// Neither happened in the time allowed.
    TimedOut,
}

impl std::fmt::Display for Readiness {
    /// Writes what happened, as the end of a sentence about the service.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Readiness::Ready => "is ready",
            Readiness::Failed => "failed",
            Readiness::TimedOut => "did not answer in time",
        };

        formatter.write_str(text)
    }
}

/// Reads the `KEY=value` lines of `systemctl show`.
pub fn parse_show(name: &str, text: &str) -> UnitState {
    let field = |key: &str| {
        text.lines()
            .filter_map(|line| line.split_once('='))
            .find(|(found, _)| *found == key)
            .map(|(_, value)| value.to_string())
            .unwrap_or_default()
    };

    UnitState {
        name: name.to_string(),
        load_state: field("LoadState"),
        active_state: field("ActiveState"),
        memory: field("MemoryCurrent").parse().ok(),
        memory_max: field("MemoryMax").parse().ok(),
        restarts: field("NRestarts").parse().unwrap_or(0),
        container: None,
    }
}

/// The service names in `systemctl list-units --plain --no-legend` output.
pub fn parse_unit_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(|unit| unit.trim_end_matches(".service").to_string())
        .collect()
}

/// The lines of a log that report an error.
pub fn error_lines(log: &str) -> Vec<&str> {
    let Some(pattern) = ERROR_LINE.as_ref() else {
        return Vec::new();
    };

    log.lines().filter(|line| pattern.is_match(line)).collect()
}

/// The model ids in an OpenAI-style `/v1/models` reply.
pub fn parse_model_ids(text: &str) -> Vec<String> {
    let reply: Value = serde_json::from_str(text).unwrap_or(Value::Null);

    reply
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

/// Runs `systemctl --user` with `arguments` and returns what it printed.
pub fn systemctl(arguments: &[&str]) -> Outcome<String> {
    run("systemctl", &[&["--user"], arguments].concat())
}

/// The state of one service.
pub fn unit_state(name: &str) -> Outcome<UnitState> {
    let text = systemctl(&[
        "show",
        "-p",
        "LoadState,ActiveState,MemoryCurrent,MemoryMax,NRestarts",
        name,
    ])?;

    Ok(parse_show(name, &text))
}

/// Every `thor-*` service systemd knows, loaded or not.
pub fn thor_units() -> Outcome<Vec<String>> {
    let text = systemctl(&[
        "list-units",
        "--all",
        "--type=service",
        "--plain",
        "--no-legend",
        "thor-*",
    ])?;

    Ok(parse_unit_list(&text))
}

/// The model ids a server on `127.0.0.1:port` lists; empty when it does not answer.
pub fn served_models(port: &str) -> Vec<String> {
    let client = web::client(Duration::from_secs(2), Duration::from_secs(5));
    let url = format!("http://127.0.0.1:{port}/v1/models");

    web::get_text(&client, &url, None)
        .map(|text| parse_model_ids(&text))
        .unwrap_or_default()
}

/// Starts `journalctl --user -u UNIT -f` with its output on this terminal, from a moment
/// given in seconds since 1970.
pub fn follow_log(unit: &str, seconds_since_epoch: u64) -> Outcome<Child> {
    let since = format!("@{seconds_since_epoch}");

    Command::new("journalctl")
        .args(["--user", "-u", unit, "--since", &since, "-f", "-o", "cat"])
        .stdin(Stdio::null())
        .spawn()
        .map_err(|source| ThorError::Program {
            program: "journalctl".to_string(),
            source,
        })
}

/// The service's log since a moment given in seconds since 1970.
pub fn log_since(unit: &str, seconds_since_epoch: u64) -> Outcome<String> {
    let since = format!("@{seconds_since_epoch}");

    run(
        "journalctl",
        &[
            "--user",
            "-u",
            unit,
            "--since",
            &since,
            "--no-pager",
            "-o",
            "cat",
        ],
    )
}

/// Waits until the server lists `model`, the service fails, or `timeout` passes.
/// A restart by systemd counts as a failure: a build that runs out of memory is restarted
/// over and over, and each attempt takes memory from everything else.
pub fn wait_until_ready(
    unit: &str,
    port: &str,
    model: &str,
    timeout: Duration,
) -> Outcome<Readiness> {
    let deadline = Instant::now() + timeout;
    let restarts_before = unit_state(unit)?.restarts;

    while Instant::now() < deadline {
        if let Some(readiness) = check_once(unit, port, model, restarts_before)? {
            return Ok(readiness);
        }

        thread::sleep(POLL_INTERVAL);
    }

    Ok(Readiness::TimedOut)
}

/// One look at the service and the server: `None` while it is still starting.
fn check_once(
    unit: &str,
    port: &str,
    model: &str,
    restarts_before: u64,
) -> Outcome<Option<Readiness>> {
    let state = unit_state(unit)?;
    let failed = state.has_failed_since(restarts_before);
    let listed = served_models(port).iter().any(|id| id == model);

    Ok(match (listed, failed) {
        (true, _) => Some(Readiness::Ready),
        (false, true) => Some(Readiness::Failed),
        (false, false) => None,
    })
}

/// Runs a program and returns its standard output, or its standard error as the failure.
pub fn run(program: &str, arguments: &[&str]) -> Outcome<String> {
    let output = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .map_err(|source| ThorError::Program {
            program: program.to_string(),
            source,
        })?;

    if !output.status.success() {
        return Err(ThorError::ProgramFailed {
            program: format!("{program} {}", arguments.join(" ")),
            output: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
