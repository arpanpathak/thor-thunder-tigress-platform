//! Whether each server answers, and how much memory the machine has left.
//!
//! A server is up when its health path answers 200 within two seconds. Memory
//! and load come from `/proc`; on a machine without it they read as zero.

use std::time::{Duration, Instant};

use serde::Serialize;
use ureq::{Agent, config::Config};

use crate::cli::Target;

/// How long a health check may take.
const TIMEOUT: Duration = Duration::from_secs(2);

/// One server's health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Health {
    /// The name the dashboard shows.
    pub name: String,
    /// `host:port`.
    pub address: String,
    /// Whether it answered 200.
    pub up: bool,
    /// How long the check took, in milliseconds.
    pub latency_ms: u64,
}

/// The machine's memory and load.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Machine {
    /// Memory in all, in MiB.
    pub memory_total_mb: u64,
    /// Memory a new process could still get, in MiB.
    pub memory_available_mb: u64,
    /// The one-minute load average.
    pub load: f64,
}

/// Checks every target, one after another.
#[must_use]
pub fn check_all(targets: &[Target]) -> Vec<Health> {
    let config: Config = Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build();
    let agent: Agent = config.into();

    targets.iter().map(|target| check(&agent, target)).collect()
}

fn check(agent: &Agent, target: &Target) -> Health {
    let started = Instant::now();
    let url = format!("http://{}{}", target.address, target.path);
    let up = agent
        .get(&url)
        .call()
        .is_ok_and(|response| response.status() == 200);

    Health {
        name: target.name.clone(),
        address: target.address.clone(),
        up,
        latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

/// The memory and load of this machine.
#[must_use]
pub fn machine() -> Machine {
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let loadavg = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();

    read_machine(&meminfo, &loadavg)
}

/// The memory in `meminfo` and the load in `loadavg`, as `/proc` writes them.
fn read_machine(meminfo: &str, loadavg: &str) -> Machine {
    Machine {
        memory_total_mb: kilobytes(meminfo, "MemTotal:") / 1024,
        memory_available_mb: kilobytes(meminfo, "MemAvailable:") / 1024,
        load: loadavg
            .split_whitespace()
            .next()
            .and_then(|first| first.parse().ok())
            .unwrap_or(0.0),
    }
}

fn kilobytes(meminfo: &str, name: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|number| number.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use anyhow::Result;

    use super::*;

    /// A server that answers one request with `status`, and its address.
    fn answering(status: &'static str) -> Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();

        thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            let mut request = [0u8; 1024];
            let _read = stream.read(&mut request)?;

            stream.write_all(format!("HTTP/1.1 {status}\r\ncontent-length: 2\r\n\r\nok").as_bytes())
        });

        Ok(address)
    }

    fn target(name: &str, address: &str) -> Target {
        Target {
            name: name.to_string(),
            address: address.to_string(),
            path: "/health",
        }
    }

    #[test]
    fn a_server_is_up_only_when_it_answers_200() -> Result<()> {
        let fine = answering("200 OK")?;
        let broken = answering("500 Internal Server Error")?;
        let closed = TcpListener::bind("127.0.0.1:0")?.local_addr()?.to_string();

        let health = check_all(&[
            target("fine", &fine),
            target("broken", &broken),
            target("gone", &closed),
        ]);

        assert_eq!(
            health.iter().map(|one| one.up).collect::<Vec<_>>(),
            [true, false, false]
        );
        assert_eq!(health[0].name, "fine");
        assert_eq!(health[0].address, fine);

        Ok(())
    }

    #[test]
    fn reads_memory_and_load_as_proc_writes_them() {
        let meminfo =
            "MemTotal:       125748000 kB\nMemFree:  1 kB\nMemAvailable:    95416000 kB\n";

        let machine = read_machine(meminfo, "0.52 0.40 0.31 1/200 42\n");

        assert_eq!(machine.memory_total_mb, 122_800);
        assert_eq!(machine.memory_available_mb, 93_179);
        assert!((machine.load - 0.52).abs() < f64::EPSILON);
        assert_eq!(read_machine("", ""), Machine::default());
    }

    #[test]
    fn this_machine_has_some_memory() {
        assert!(machine().memory_total_mb > 0);
    }
}
