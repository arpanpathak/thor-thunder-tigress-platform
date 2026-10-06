//! The server's settings, from the command line and the key file.
//!
//! ```text
//! thor-tigress-agent [--listen 127.0.0.1:8080] [--model 127.0.0.1:8079]
//!                    [--search 127.0.0.1:8888] [--web DIR] [--key-file FILE]
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{error::AgentError, upstream::Endpoint};

/// The model server and the search engine.
pub struct Upstreams {
    /// llama-server, called with the access key when there is one.
    pub model: Endpoint,
    /// SearXNG, called without a key.
    pub search: Endpoint,
}

/// The settings of a running server.
pub struct Config {
    /// The address to listen on, `host:port`.
    pub listen: String,
    /// The folder holding the page, the About page and the art.
    pub web: PathBuf,
    /// The access key; `None` lets every request through.
    pub key: Option<String>,
    /// Where model and search requests go.
    pub upstreams: Upstreams,
}

impl Config {
    /// Reads the options in `arguments` (without the program name) and the
    /// key file they point to.
    ///
    /// # Errors
    ///
    /// `AgentError::Config` for an unknown option or one without a value.
    pub fn from_args(arguments: impl IntoIterator<Item = String>) -> Result<Self, AgentError> {
        let options = Options::parse(arguments)?;
        let key = read_key(&options.key_file);
        let authorization = key.as_ref().map(|key| format!("Bearer {key}"));
        Ok(Config {
            listen: options.listen,
            web: options.web,
            key,
            upstreams: Upstreams {
                model: Endpoint::new(options.model, authorization),
                search: Endpoint::new(options.search, None),
            },
        })
    }

    /// Whether a request carrying the `Authorization` value `sent` may use
    /// the model.
    #[must_use]
    pub fn admits(&self, sent: Option<&str>) -> bool {
        match (&self.key, sent) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(key), Some(sent)) => sent
                .strip_prefix("Bearer ")
                .is_some_and(|given| same_secret(given, key)),
        }
    }
}

/// The options as given, before the key file is read.
struct Options {
    listen: String,
    model: String,
    search: String,
    web: PathBuf,
    key_file: PathBuf,
}

impl Default for Options {
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        Options {
            listen: "127.0.0.1:8080".to_string(),
            model: "127.0.0.1:8079".to_string(),
            search: "127.0.0.1:8888".to_string(),
            web: PathBuf::from("."),
            key_file: home.join(".config/thor-chat/api-key"),
        }
    }
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, AgentError> {
        let mut options = Options::default();
        let mut arguments = arguments.into_iter();
        while let Some(flag) = arguments.next() {
            let Some(value) = arguments.next() else {
                return Err(AgentError::Config(format!("{flag} needs a value")));
            };
            options.set(&flag, value)?;
        }
        Ok(options)
    }

    fn set(&mut self, flag: &str, value: String) -> Result<(), AgentError> {
        match flag {
            "--listen" => self.listen = value,
            "--model" => self.model = value,
            "--search" => self.search = value,
            "--web" => self.web = PathBuf::from(value),
            "--key-file" => self.key_file = PathBuf::from(value),
            unknown => return Err(AgentError::Config(format!("unknown option {unknown}"))),
        }
        Ok(())
    }
}

/// The key in `path`; `None` when the file is missing or empty, which means
/// no key is required.
fn read_key(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let key = text.trim();
    (!key.is_empty()).then(|| key.to_string())
}

/// Compares two secrets in time that depends only on their length, so the
/// response time doesn't reveal how much of a guess was right.
fn same_secret(given: &str, expected: &str) -> bool {
    let difference = given
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b));
    given.len() == expected.len() && difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    fn with_key(key: Option<&str>) -> Result<Config, AgentError> {
        let mut config = Config::from_args(args(&["--key-file", "/nonexistent"]))?;
        config.key = key.map(ToString::to_string);
        Ok(config)
    }

    #[test]
    fn defaults_point_at_localhost() -> Result<(), AgentError> {
        let config = Config::from_args(args(&["--key-file", "/nonexistent"]))?;
        assert_eq!(config.listen, "127.0.0.1:8080");
        assert_eq!(config.upstreams.model.address(), "127.0.0.1:8079");
        assert_eq!(config.upstreams.search.address(), "127.0.0.1:8888");
        assert_eq!(config.key, None);
        Ok(())
    }

    #[test]
    fn options_override_defaults() -> Result<(), AgentError> {
        let config = Config::from_args(args(&[
            "--listen", "0.0.0.0:9000", "--model", "m:1", "--search", "s:2", "--web", "/srv", "--key-file", "/none",
        ]))?;
        assert_eq!(config.listen, "0.0.0.0:9000");
        assert_eq!(config.upstreams.model.address(), "m:1");
        assert_eq!(config.upstreams.search.address(), "s:2");
        assert_eq!(config.web, PathBuf::from("/srv"));
        Ok(())
    }

    #[test]
    fn rejects_unknown_and_incomplete_options() {
        let unknown = Config::from_args(args(&["--port", "1"]));
        let incomplete = Config::from_args(args(&["--listen"]));
        assert!(unknown.is_err_and(|error| error.to_string() == "config: unknown option --port"));
        assert!(incomplete.is_err_and(|error| error.to_string() == "config: --listen needs a value"));
    }

    #[test]
    fn reads_the_key_file_trimmed() -> Result<(), AgentError> {
        let path = std::env::temp_dir().join(format!("thor-key-{}", std::process::id()));
        fs::write(&path, "  secret\n")?;
        let config = Config::from_args(args(&["--key-file", &path.to_string_lossy()]));
        fs::remove_file(&path)?;
        let config = config?;
        assert_eq!(config.key.as_deref(), Some("secret"));
        assert_eq!(config.upstreams.model.authorization(), Some("Bearer secret"));
        assert_eq!(config.upstreams.search.authorization(), None);
        Ok(())
    }

    #[test]
    fn an_empty_key_file_means_no_key() -> Result<(), AgentError> {
        let path = std::env::temp_dir().join(format!("thor-empty-key-{}", std::process::id()));
        fs::write(&path, "\n")?;
        let key = read_key(&path);
        fs::remove_file(&path)?;
        assert_eq!(key, None);
        Ok(())
    }

    #[test]
    fn admits_only_the_right_bearer_key() -> Result<(), AgentError> {
        let open = with_key(None)?;
        let locked = with_key(Some("k3y"))?;
        assert!(open.admits(None));
        assert!(locked.admits(Some("Bearer k3y")));
        assert!(!locked.admits(None));
        assert!(!locked.admits(Some("Bearer k3")));
        assert!(!locked.admits(Some("Bearer k3yy")));
        assert!(!locked.admits(Some("k3y")));
        Ok(())
    }

    #[test]
    fn secrets_compare_by_whole_value() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abd", "abc"));
        assert!(!same_secret("ab", "abc"));
        assert!(!same_secret("", "abc"));
    }
}
