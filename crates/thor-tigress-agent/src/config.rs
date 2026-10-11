//! The server's settings, from the command line, the key file and the keyring.
//!
//! ```text
//! thor-tigress-agent [--listen 127.0.0.1:8080] [--model 127.0.0.1:8079]
//!                    [--engine MODEL=HOST:PORT]... [--search 127.0.0.1:8888]
//!                    [--web DIR] [--key-file FILE]
//!                    [--keyring FILE] [--keyring-passphrase-file FILE]
//! ```
//!
//! `--model` is llama-server, which answers for the models it has loaded.
//! Each `--engine` names one model and the TensorRT Edge-LLM server holding
//! it. A request goes to the server that has the model it names. A request
//! naming a model nobody has, or none, goes to whichever of them comes first
//! on the command line; with neither option that is llama-server on its
//! usual port.
//!
//! `--key-file` is the one key this server sends to llama-server, and it lets
//! the operator in. When `--keyring` names an encrypted registry, the
//! personal keys in it are what visitors may use. The
//! keyring's passphrase comes from `--keyring-passphrase-file` or the
//! `THOR_KEYRING_PASSPHRASE` environment variable.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::SystemTime,
};

use thor_tigress_keyring::{error::KeyringError, store::Keyring};
use zeroize::Zeroizing;

use crate::{
    error::{AgentError, Outcome},
    web::http::{HttpWeb, Web},
    engines::{models, upstream::Endpoint},
};

/// Where the server listens unless `--listen` says otherwise.
const DEFAULT_LISTEN: &str = "127.0.0.1:8080";

/// Where llama-server listens.
const DEFAULT_MODEL: &str = "127.0.0.1:8079";

/// Where SearXNG listens.
const DEFAULT_SEARCH: &str = "127.0.0.1:8888";

/// The key file, relative to `$HOME`.
const KEY_FILE: &str = ".config/thor-chat/api-key";

/// Where the keyring's passphrase is read from unless a file is given.
const KEYRING_ENV: &str = "THOR_KEYRING_PASSPHRASE";

/// How a key is sent in an `Authorization` header.
pub const BEARER: &str = "Bearer ";

/// The model servers and the search engine.
pub struct Upstreams {
    /// The server that answers a request naming a model nobody has.
    pub default: Engine,
    /// The other servers, each answering requests for the models it has.
    pub others: Vec<Engine>,
    /// SearXNG, called without a key.
    pub search: Endpoint,
    /// The web, for the fetch tool: one HTTPS client with the address checks.
    pub web: Box<dyn Web>,
}

/// A server that answers for models. The two kinds read a request
/// differently, so every caller that shapes one matches on the kind.
#[derive(Debug, Clone)]
pub enum Engine {
    /// One model on a TensorRT Edge-LLM server, called without a key: it
    /// listens on localhost only.
    Edge {
        /// The model id requests name.
        model: String,
        /// Where it is served.
        endpoint: Endpoint,
    },
    /// llama-server in router mode, called with the access key when there is
    /// one. It answers for whichever models it has loaded.
    Llama {
        /// Where it is served.
        endpoint: Endpoint,
    },
}

impl Engine {
    /// The Edge-LLM server holding `model` at `address` (`host:port`).
    #[must_use]
    pub fn edge(model: String, address: String) -> Self {
        Engine::Edge {
            model,
            endpoint: Endpoint::new(address, None),
        }
    }

    /// llama-server at `address`, sent `authorization` when there is one.
    #[must_use]
    pub fn llama(address: String, authorization: Option<String>) -> Self {
        Engine::Llama {
            endpoint: Endpoint::new(address, authorization),
        }
    }

    /// Where the server is called.
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        match self {
            Engine::Edge { endpoint, .. } | Engine::Llama { endpoint } => endpoint,
        }
    }

    /// What the server is called in the startup line.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Engine::Edge { model, .. } => model,
            Engine::Llama { .. } => "llama-server",
        }
    }

    /// Whether the server has `model` ready: an Edge-LLM server by its one
    /// name, llama-server by asking which models it has loaded.
    fn serves(&self, model: Option<&str>) -> bool {
        match (self, model) {
            (_, None) => false,
            (Engine::Edge { model: served, .. }, Some(wanted)) => served == wanted,
            (Engine::Llama { endpoint }, Some(wanted)) => models::loaded_names(endpoint)
                .iter()
                .any(|name| name == wanted),
        }
    }
}

impl Upstreams {
    /// Every server, the default first.
    pub fn engines(&self) -> impl Iterator<Item = &Engine> {
        std::iter::once(&self.default).chain(&self.others)
    }

    /// The server for a request naming `model`: the one that has it, else the
    /// default. A coding tool sends the name it was built around, such as a
    /// hosted model's, and still has to be answered.
    #[must_use]
    pub fn serving(&self, model: Option<&str>) -> &Engine {
        self.engines()
            .find(|engine| engine.serves(model))
            .unwrap_or(&self.default)
    }
}

/// The people whose personal keys the chat accepts, read from the encrypted
/// keyring and reloaded when the file changes.
pub struct People {
    path: PathBuf,
    passphrase: Zeroizing<String>,
    state: Mutex<Loaded>,
}

/// The active keys, and the keyring file's timestamp when they were read.
#[derive(Default)]
struct Loaded {
    keys: Vec<String>,
    modified: Option<SystemTime>,
}

impl People {
    /// Reads the keyring at `path`, so a wrong passphrase or a missing file is
    /// caught at startup, not on the first visitor.
    ///
    /// # Errors
    ///
    /// [`AgentError::Keyring`] when the file cannot be read or opened.
    pub fn new(path: PathBuf, passphrase: Zeroizing<String>) -> Outcome<Self> {
        let keyring = Keyring::open(&path, &passphrase)?;
        let loaded = Loaded {
            keys: keyring.active_keys(),
            modified: modified(&path),
        };

        Ok(People {
            path,
            passphrase,
            state: Mutex::new(loaded),
        })
    }

    /// Whether `sent` carries the key of someone the keyring calls active.
    #[must_use]
    pub fn admit(&self, sent: Option<&str>) -> bool {
        let Some(sent) = sent.and_then(|value| value.strip_prefix(BEARER)) else {
            return false;
        };

        let mut loaded = self.lock();

        if let Err(error) = self.refresh(&mut loaded) {
            eprintln!("keyring: {error}");
        }

        loaded.keys.iter().any(|key| same_secret(sent, key))
    }

    /// Records a request from the registration form, so the author can approve
    /// it with `thor-tigress-keyring approve`.
    ///
    /// # Errors
    ///
    /// [`KeyringError`] when the keyring cannot be read, or when the name or
    /// email is not acceptable.
    pub fn request(&self, name: &str, email: &str) -> Result<(), KeyringError> {
        let mut loaded = self.lock();
        let mut keyring = Keyring::open(&self.path, &self.passphrase)?;
        keyring.request(name, email)?;
        loaded.modified = None;

        Ok(())
    }

    fn refresh(&self, loaded: &mut Loaded) -> Result<(), KeyringError> {
        let modified = modified(&self.path);

        if modified.is_some() && modified == loaded.modified {
            return Ok(());
        }

        let keyring = Keyring::open(&self.path, &self.passphrase)?;
        loaded.keys = keyring.active_keys();
        loaded.modified = modified;

        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, Loaded> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The settings of a running server.
pub struct Config {
    /// The address to listen on, `host:port`.
    pub listen: String,
    /// The folder holding the page, the About page and the art.
    pub web: PathBuf,
    /// The single access key; `None` lets every request through when there is
    /// no keyring either.
    pub key: Option<String>,
    /// The personal keys in the encrypted keyring, when one is configured.
    pub people: Option<People>,
    /// Where model and search requests go.
    pub upstreams: Upstreams,
}

impl Config {
    /// Reads the options in `arguments` (without the program name), the key
    /// file they point to, and the keyring they name.
    ///
    /// # Errors
    ///
    /// `AgentError::Config` for an unknown option or one without a value, and
    /// `AgentError::Keyring` when the keyring cannot be opened.
    pub fn from_args(arguments: impl IntoIterator<Item = String>) -> Outcome<Self> {
        let options = Options::parse(arguments)?;
        let key = read_key(&options.key_file);
        let authorization = key.as_ref().map(|key| format!("{BEARER}{key}"));
        let mut engines = options
            .servers
            .iter()
            .map(|wanted| wanted.engine(authorization.clone()));
        let default = engines
            .next()
            .unwrap_or_else(|| Engine::llama(DEFAULT_MODEL.to_string(), authorization.clone()));

        let people = match &options.keyring {
            Some(path) => {
                let passphrase = keyring_passphrase(&options, std::env::var(KEYRING_ENV).ok())?;

                Some(People::new(path.clone(), passphrase)?)
            }
            None => None,
        };

        Ok(Config {
            listen: options.listen,
            web: options.web,
            key,
            people,
            upstreams: Upstreams {
                default,
                others: engines.collect(),
                search: Endpoint::new(options.search, None),
                web: Box::new(HttpWeb::new()),
            },
        })
    }

    /// Whether a request carrying the `Authorization` value `sent` may use
    /// the model: a personal key from the keyring, or the single key.
    #[must_use]
    pub fn admits(&self, sent: Option<&str>) -> bool {
        if self
            .people
            .as_ref()
            .is_some_and(|people| people.admit(sent))
        {
            return true;
        }

        match (&self.key, sent) {
            (None, _) => self.people.is_none(),
            (Some(_), None) => false,
            (Some(key), Some(sent)) => sent
                .strip_prefix(BEARER)
                .is_some_and(|given| same_secret(given, key)),
        }
    }

    /// Records a request for access from the registration form.
    ///
    /// # Errors
    ///
    /// [`AgentError::BadRequest`] when no keyring is configured (registration
    /// is closed), and [`AgentError::Keyring`] when the keyring cannot be
    /// written.
    pub fn request_access(&self, name: &str, email: &str) -> Outcome {
        match &self.people {
            Some(people) => {
                people.request(name, email)?;

                Ok(())
            }
            None => Err(AgentError::bad_request("registration is closed")),
        }
    }
}

/// A model server as the command line names it, before the key file is read.
enum Wanted {
    /// `--model HOST:PORT`.
    Llama(String),
    /// `--engine MODEL=HOST:PORT`.
    Edge(String, String),
}

impl Wanted {
    fn engine(&self, authorization: Option<String>) -> Engine {
        match self {
            Wanted::Llama(address) => Engine::llama(address.clone(), authorization),
            Wanted::Edge(model, address) => Engine::edge(model.clone(), address.clone()),
        }
    }
}

/// The options as given, before the key file is read.
struct Options {
    listen: String,
    servers: Vec<Wanted>,
    search: String,
    web: PathBuf,
    key_file: PathBuf,
    keyring: Option<PathBuf>,
    keyring_passphrase_file: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();

        Options {
            listen: DEFAULT_LISTEN.to_string(),
            servers: Vec::new(),
            search: DEFAULT_SEARCH.to_string(),
            web: PathBuf::from("."),
            key_file: home.join(KEY_FILE),
            keyring: None,
            keyring_passphrase_file: None,
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
            "--model" => self.servers.push(Wanted::Llama(value)),
            "--engine" => self.servers.push(engine(&value)?),
            "--search" => self.search = value,
            "--web" => self.web = PathBuf::from(value),
            "--key-file" => self.key_file = PathBuf::from(value),
            "--keyring" => self.keyring = Some(PathBuf::from(value)),
            "--keyring-passphrase-file" => {
                self.keyring_passphrase_file = Some(PathBuf::from(value))
            }
            unknown => return Err(AgentError::Config(format!("unknown option {unknown}"))),
        }

        Ok(())
    }
}

/// Reads `MODEL=HOST:PORT`.
fn engine(value: &str) -> Outcome<Wanted> {
    match value.split_once('=') {
        Some((model, address)) if !model.is_empty() && !address.is_empty() => {
            Ok(Wanted::Edge(model.to_string(), address.to_string()))
        }
        _ => Err(AgentError::Config(format!(
            "--engine {value}: expected MODEL=HOST:PORT"
        ))),
    }
}

/// The keyring's passphrase: the file's first line, or the environment.
fn keyring_passphrase(options: &Options, from_env: Option<String>) -> Outcome<Zeroizing<String>> {
    if let Some(path) = &options.keyring_passphrase_file {
        let text = fs::read_to_string(path)
            .map_err(|error| AgentError::Config(format!("{}: {error}", path.display())))?;
        let line = text.lines().next().unwrap_or_default().trim();

        if line.is_empty() {
            return Err(AgentError::Config(format!(
                "{}: the passphrase is empty",
                path.display()
            )));
        }

        return Ok(Zeroizing::new(line.to_string()));
    }

    match from_env {
        Some(value) if !value.is_empty() => Ok(Zeroizing::new(value)),
        _ => Err(AgentError::Config(
            "--keyring needs --keyring-passphrase-file or THOR_KEYRING_PASSPHRASE".to_string(),
        )),
    }
}

/// The key in `path`; `None` when the file is missing or empty, which means
/// no key is required.
fn read_key(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let key = text.trim();

    (!key.is_empty()).then(|| key.to_string())
}

/// When `path` was last changed, or `None` when it is not there.
fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
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
mod tests;
