//! The Docker container vLLM runs in.
//!
//! `thor-tigress-serve` starts vLLM with `docker run --rm --name thor-vllm` from the
//! `thor-vllm` user service. The container's memory is not in that service's cgroup, and
//! stopping the service leaves the container running, so `thor` asks Docker directly.
//! A session started before the user joined the `docker` group cannot reach the Docker
//! daemon, so `thor` then runs `docker` through `sg docker -c`, as `thor-tigress-serve` does.

use std::{
    thread,
    time::{Duration, Instant},
};

use crate::{
    error::{Outcome, ThorError},
    service,
};

/// How long `thor stop` waits for a stopped container to disappear.
const STOP_TIMEOUT: Duration = Duration::from_secs(120);

/// Sizes Docker prints, with their bytes: binary units first, then decimal ones.
const UNITS: [(&str, f64); 10] = [
    ("TiB", 1_099_511_627_776.0),
    ("GiB", 1_073_741_824.0),
    ("MiB", 1_048_576.0),
    ("KiB", 1024.0),
    ("TB", 1e12),
    ("GB", 1e9),
    ("MB", 1e6),
    ("kB", 1e3),
    ("KB", 1e3),
    ("B", 1.0),
];

/// Runs `docker` with `arguments`, and through `sg docker -c` when `docker` alone fails.
pub fn docker(arguments: &[&str]) -> Outcome<String> {
    service::run("docker", arguments).or_else(|_direct_failure| {
        let command = shell_words(&[&["docker"], arguments].concat());

        service::run("sg", &["docker", "-c", &command])
    })
}

/// The arguments as one shell command line, each in single quotes.
pub fn shell_words(arguments: &[&str]) -> String {
    arguments
        .iter()
        .map(|argument| format!("'{}'", argument.replace('\'', r"'\''")))
        .collect::<Vec<String>>()
        .join(" ")
}

/// The memory a running container uses, from `docker stats`; `None` when it is not running
/// or Docker cannot be asked.
pub fn container_memory(name: &str) -> Option<u64> {
    let usage = docker(&["stats", "--no-stream", "--format", "{{.MemUsage}}", name]).ok()?;

    parse_memory_usage(&usage)
}

/// The used part of `docker stats`' `MemUsage`, such as `98.5GiB / 122.8GiB`, in bytes.
pub fn parse_memory_usage(text: &str) -> Option<u64> {
    let used = text.split('/').next()?.trim();
    let (unit, scale) = UNITS.iter().find(|(unit, _scale)| used.ends_with(unit))?;
    let number: f64 = used.trim_end_matches(unit).trim().parse().ok()?;

    Some((number * scale) as u64)
}

/// Whether a container with exactly this name is running.
pub fn is_running(name: &str) -> bool {
    let filter = format!("name=^/{name}$");

    docker(&["ps", "--quiet", "--filter", &filter]).is_ok_and(|ids| !ids.trim().is_empty())
}

/// Stops the container when it is running, and waits until Docker no longer lists it.
/// Returns whether there was one to stop.
pub fn stop_container(name: &str) -> Outcome<bool> {
    if !is_running(name) {
        return Ok(false);
    }

    println!("Stopping the Docker container {name}…");
    docker(&["stop", name])?;
    wait_until_gone(name)?;

    Ok(true)
}

/// Waits until the container is gone, for at most [`STOP_TIMEOUT`].
fn wait_until_gone(name: &str) -> Outcome<()> {
    let deadline = Instant::now() + STOP_TIMEOUT;

    while is_running(name) {
        if Instant::now() > deadline {
            return Err(ThorError::Condition(format!(
                "{name} is still running after {} s; see docker ps",
                STOP_TIMEOUT.as_secs()
            )));
        }

        thread::sleep(Duration::from_secs(1));
    }

    Ok(())
}
