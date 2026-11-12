//! The command line.
//!
//! ```text
//! thor-tigress-telemetry [--log FILE] [--openobserve URL] [--org NAME]
//!                        [--user EMAIL] [--password-file FILE]
//!                        [--agent HOST:PORT] [--engine NAME=HOST:PORT]...
//!                        [--every SECONDS] [--rounds N]
//! ```
//!
//! `--log` is the agent's request log. `--agent` and each `--engine` are
//! checked for health on every round. `--rounds` ships that many rounds and
//! exits; without it the shipper runs until it is stopped.

use std::path::PathBuf;

use crate::error::{Outcome, TelemetryError};

/// The request log, relative to `$HOME`.
const LOG: &str = ".local/state/thor-tigress/requests.jsonl";

/// OpenObserve's password, relative to `$HOME`.
const PASSWORD: &str = ".config/thor-chat/openobserve-password";

/// What the help text says.
pub const USAGE: &str = "thor-tigress-telemetry [--log FILE] [--openobserve URL] [--org NAME] \
[--user EMAIL] [--password-file FILE] [--agent HOST:PORT] [--engine NAME=HOST:PORT]... \
[--every SECONDS] [--rounds N]";

/// A server checked for health, by the name the dashboard shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The name shown on the dashboard.
    pub name: String,
    /// `host:port`.
    pub address: String,
    /// The path that answers when the server is up.
    pub path: &'static str,
}

/// The options as given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The agent's request log.
    pub log: PathBuf,
    /// OpenObserve's address, such as `http://127.0.0.1:5080`.
    pub openobserve: String,
    /// The OpenObserve organization the streams belong to.
    pub org: String,
    /// The OpenObserve user.
    pub user: String,
    /// The file holding the user's password.
    pub password_file: PathBuf,
    /// The agent and the engines to check.
    pub targets: Vec<Target>,
    /// Seconds between rounds.
    pub every: u64,
    /// How many rounds to ship before exiting; `None` runs until stopped.
    pub rounds: Option<u64>,
}

impl Options {
    /// Reads `arguments`, without the program name.
    ///
    /// # Errors
    ///
    /// [`TelemetryError::Usage`] for an unknown option, a missing value, an
    /// engine that is not `NAME=HOST:PORT`, or an interval that is not a number.
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Outcome<Self> {
        let mut options = Options::with_home(std::env::var_os("HOME").map(PathBuf::from));
        let mut arguments = arguments.into_iter();

        while let Some(flag) = arguments.next() {
            options.set(&flag, &mut arguments)?;
        }

        Ok(options)
    }

    fn with_home(home: Option<PathBuf>) -> Self {
        let home = home.unwrap_or_default();

        Options {
            log: home.join(LOG),
            openobserve: "http://127.0.0.1:5080".to_string(),
            org: "default".to_string(),
            user: "admin@thor.local".to_string(),
            password_file: home.join(PASSWORD),
            targets: vec![agent("127.0.0.1:8080")],
            every: 5,
            rounds: None,
        }
    }

    fn set(&mut self, flag: &str, rest: &mut impl Iterator<Item = String>) -> Outcome {
        let value = rest
            .next()
            .ok_or_else(|| usage(format!("{flag} needs a value")))?;

        self.set_value(flag, value)
    }

    fn set_value(&mut self, flag: &str, value: String) -> Outcome {
        match flag {
            "--log" => self.log = PathBuf::from(value),
            "--openobserve" => self.openobserve = value,
            "--org" => self.org = value,
            "--user" => self.user = value,
            "--password-file" => self.password_file = PathBuf::from(value),
            "--agent" => self.targets[0] = agent(&value),
            "--engine" => self.targets.push(engine(&value)?),
            "--every" => self.every = positive(flag, &value)?,
            "--rounds" => self.rounds = Some(positive(flag, &value)?),
            unknown => return Err(usage(format!("unknown option {unknown}"))),
        }

        Ok(())
    }
}

fn agent(address: &str) -> Target {
    Target {
        name: "thor-tigress-agent".to_string(),
        address: address.to_string(),
        path: "/health",
    }
}

fn engine(value: &str) -> Outcome<Target> {
    let (name, address) = value
        .split_once('=')
        .filter(|(name, address)| !name.is_empty() && !address.is_empty())
        .ok_or_else(|| usage(format!("--engine {value}: expected NAME=HOST:PORT")))?;

    Ok(Target {
        name: name.to_string(),
        address: address.to_string(),
        path: "/v1/models",
    })
}

fn positive(flag: &str, value: &str) -> Outcome<u64> {
    value
        .parse()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| usage(format!("{flag} {value}: expected a number above 0")))
}

fn usage(reason: String) -> TelemetryError {
    TelemetryError::Usage(format!("{reason}\nusage: {USAGE}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn defaults_point_at_the_agent_its_log_and_a_local_openobserve() -> Outcome {
        let options = Options::parse(args(&[]))?;
        let homeless = Options::with_home(None);

        assert!(options.log.ends_with(LOG));
        assert!(options.password_file.ends_with(PASSWORD));
        assert_eq!(options.openobserve, "http://127.0.0.1:5080");
        assert_eq!(options.targets, vec![agent("127.0.0.1:8080")]);
        assert_eq!((options.every, options.rounds), (5, None));
        assert_eq!(homeless.log, PathBuf::from(LOG));

        Ok(())
    }

    #[test]
    fn reads_every_option() -> Outcome {
        let given = "--log /tmp/r.jsonl --openobserve http://10.0.0.2:5080 --org thor \
            --user ops@example.com --password-file /tmp/p --agent 10.0.0.2:8080 \
            --engine gemma=127.0.0.1:8081 --every 10 --rounds 3";
        let options = Options::parse(given.split_whitespace().map(String::from))?;

        assert_eq!(options.log, PathBuf::from("/tmp/r.jsonl"));
        assert_eq!(options.openobserve, "http://10.0.0.2:5080");
        assert_eq!(
            (options.org.as_str(), options.user.as_str()),
            ("thor", "ops@example.com")
        );
        assert_eq!(options.password_file, PathBuf::from("/tmp/p"));
        assert_eq!(options.targets[0].address, "10.0.0.2:8080");
        assert_eq!(options.targets[1].name, "gemma");
        assert_eq!((options.every, options.rounds), (10, Some(3)));

        Ok(())
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        let cases = [
            vec!["--colour", "red"],
            vec!["--log"],
            vec!["--engine", "gemma"],
            vec!["--engine", "=127.0.0.1:1"],
            vec!["--every", "0"],
            vec!["--every", "soon"],
            vec!["--rounds", "0"],
        ];

        let refused = cases.iter().map(|case| Options::parse(args(case)));

        assert!(refused.into_iter().all(|result| {
            matches!(result, Err(TelemetryError::Usage(message)) if message.contains("usage:"))
        }));
    }
}
