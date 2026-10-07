//! The server's settings, from the command line and the key file.
//!
//! ```text
//! thor-tigress-agent [--listen 127.0.0.1:8080] [--model 127.0.0.1:8079]
//!                    [--engine MODEL=HOST:PORT]... [--search 127.0.0.1:8888]
//!                    [--web DIR] [--key-file FILE]
//! ```
//!
//! `--engine` names a model another engine serves, such as TensorRT
//! Edge-LLM; requests for it go there, everything else to `--model`.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    error::{AgentError, Outcome},
    upstream::Endpoint,
};

/// Where the server listens unless `--listen` says otherwise.
const DEFAULT_LISTEN: &str = "127.0.0.1:8080";
/// Where llama-server listens.
const DEFAULT_MODEL: &str = "127.0.0.1:8079";
/// Where SearXNG listens.
const DEFAULT_SEARCH: &str = "127.0.0.1:8888";
/// The key file, relative to `$HOME`.
const KEY_FILE: &str = ".config/thor-chat/api-key";
/// How a key is sent in an `Authorization` header.
pub const BEARER: &str = "Bearer ";

/// The model servers and the search engine.
pub struct Upstreams {
    /// llama-server, called with the access key when there is one; it serves
    /// every model no engine below serves.
    pub model: Endpoint,
    /// Models served by other engines.
    pub engines: Vec<Engine>,
    /// SearXNG, called without a key.
    pub search: Endpoint,
}

/// A model served by another engine, such as TensorRT Edge-LLM.
#[derive(Debug, Clone)]
pub struct Engine {
    /// The model id requests name.
    pub model: String,
    /// Where it is served, called without a key: it listens on localhost only.
    pub endpoint: Endpoint,
}

impl Upstreams {
    /// The server for requests naming `model`: the engine serving it, else
    /// llama-server.
    #[must_use]
    pub fn serving(&self, model: Option<&str>) -> &Endpoint {
        self.engines
            .iter()
            .find(|engine| Some(engine.model.as_str()) == model)
            .map_or(&self.model, |engine| &engine.endpoint)
    }

    /// The server for the model a JSON request `body` names.
    #[must_use]
    pub fn serving_body(&self, body: &[u8]) -> &Endpoint {
        #[derive(Deserialize)]
        struct Named {
            model: Option<String>,
        }
        let named: Option<Named> = serde_json::from_slice(body).ok();
        self.serving(named.and_then(|named| named.model).as_deref())
    }
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
    pub fn from_args(arguments: impl IntoIterator<Item = String>) -> Outcome<Self> {
        let options = Options::parse(arguments)?;
        let key = read_key(&options.key_file);
        let authorization = key.as_ref().map(|key| format!("{BEARER}{key}"));
        Ok(Config {
            listen: options.listen,
            web: options.web,
            key,
            upstreams: Upstreams {
                model: Endpoint::new(options.model, authorization),
                engines: options
                    .engines
                    .into_iter()
                    .map(|(model, address)| Engine { model, endpoint: Endpoint::new(address, None) })
                    .collect(),
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
                .strip_prefix(BEARER)
                .is_some_and(|given| same_secret(given, key)),
        }
    }
}

/// The options as given, before the key file is read.
struct Options {
    listen: String,
    model: String,
    engines: Vec<(String, String)>,
    search: String,
    web: PathBuf,
    key_file: PathBuf,
}

impl Default for Options {
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        Options {
            listen: DEFAULT_LISTEN.to_string(),
            model: DEFAULT_MODEL.to_string(),
            engines: Vec::new(),
            search: DEFAULT_SEARCH.to_string(),
            web: PathBuf::from("."),
            key_file: home.join(KEY_FILE),
        }
    }
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = String>) -> Outcome<Self> {
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

    fn set(&mut self, flag: &str, value: String) -> Outcome {
        match flag {
            "--listen" => self.listen = value,
            "--model" => self.model = value,
            "--engine" => self.engines.push(engine(&value)?),
            "--search" => self.search = value,
            "--web" => self.web = PathBuf::from(value),
            "--key-file" => self.key_file = PathBuf::from(value),
            unknown => return Err(AgentError::Config(format!("unknown option {unknown}"))),
        }
        Ok(())
    }
}

/// Reads `MODEL=HOST:PORT`.
fn engine(value: &str) -> Outcome<(String, String)> {
    match value.split_once('=') {
        Some((model, address)) if !model.is_empty() && !address.is_empty() => Ok((model.to_string(), address.to_string())),
        _ => Err(AgentError::Config(format!("--engine {value}: expected MODEL=HOST:PORT"))),
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

    fn with_key(key: Option<&str>) -> Outcome<Config> {
        let mut config = Config::from_args(args(&["--key-file", "/nonexistent"]))?;
        config.key = key.map(ToString::to_string);
        Ok(config)
    }

    #[test]
    fn defaults_point_at_localhost() -> Outcome {
        let config = Config::from_args(args(&["--key-file", "/nonexistent"]))?;
        assert_eq!(config.listen, "127.0.0.1:8080");
        assert_eq!(config.upstreams.model.address(), "127.0.0.1:8079");
        assert_eq!(config.upstreams.search.address(), "127.0.0.1:8888");
        assert_eq!(config.key, None);
        Ok(())
    }

    #[test]
    fn options_override_defaults() -> Outcome {
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
    fn routes_named_models_to_their_engine() -> Outcome {
        let config = Config::from_args(args(&["--engine", "qwen=e:1", "--key-file", "/none"]))?;
        let upstreams = &config.upstreams;
        assert_eq!(upstreams.serving(Some("qwen")).address(), "e:1");
        assert_eq!(upstreams.serving(Some("nemotron")).address(), "127.0.0.1:8079");
        assert_eq!(upstreams.serving(None).address(), "127.0.0.1:8079");
        assert_eq!(upstreams.serving_body(br#"{"model":"qwen"}"#).address(), "e:1");
        assert_eq!(upstreams.serving_body(br#"{"messages":[]}"#).address(), "127.0.0.1:8079");
        assert_eq!(upstreams.serving_body(b"not json").address(), "127.0.0.1:8079");
        assert_eq!(upstreams.engines[0].endpoint.authorization(), None);
        Ok(())
    }

    #[test]
    fn rejects_an_engine_without_a_model_or_address() {
        for value in ["qwen", "=e:1", "qwen="] {
            let refused = Config::from_args(args(&["--engine", value]));
            assert!(refused.is_err_and(|error| error.to_string().ends_with("expected MODEL=HOST:PORT")), "{value}");
        }
    }

    #[test]
    fn rejects_unknown_and_incomplete_options() {
        let unknown = Config::from_args(args(&["--port", "1"]));
        let incomplete = Config::from_args(args(&["--listen"]));
        assert!(unknown.is_err_and(|error| error.to_string() == "config: unknown option --port"));
        assert!(incomplete.is_err_and(|error| error.to_string() == "config: --listen needs a value"));
    }

    #[test]
    fn reads_the_key_file_trimmed() -> Outcome {
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
    fn an_empty_key_file_means_no_key() -> Outcome {
        let path = std::env::temp_dir().join(format!("thor-empty-key-{}", std::process::id()));
        fs::write(&path, "\n")?;
        let key = read_key(&path);
        fs::remove_file(&path)?;
        assert_eq!(key, None);
        Ok(())
    }

    #[test]
    fn admits_only_the_right_bearer_key() -> Outcome {
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
