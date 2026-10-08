//! The server's settings, from the command line, the key file and the keyring.
//!
//! ```text
//! thor-tigress-agent [--listen 127.0.0.1:8080] [--model 127.0.0.1:8079]
//!                    [--engine MODEL=HOST:PORT]... [--search 127.0.0.1:8888]
//!                    [--web DIR] [--key-file FILE]
//!                    [--keyring FILE] [--keyring-passphrase-file FILE]
//! ```
//!
//! `--engine` names a model another engine serves, such as TensorRT
//! Edge-LLM; requests for it go there, everything else to `--model`.
//!
//! `--key-file` is the one key this server sends to llama-server. When
//! `--keyring` names an encrypted registry, the personal keys in it are what
//! visitors may use, and `--key-file` still lets the operator in. The
//! keyring's passphrase comes from `--keyring-passphrase-file` or the
//! `THOR_KEYRING_PASSPHRASE` environment variable.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::SystemTime,
};

use serde::Deserialize;
use thor_tigress_keyring::{error::KeyringError, store::Keyring};
use zeroize::Zeroizing;

use crate::{
    error::{AgentError, Outcome},
    http::{HttpWeb, Web},
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

/// Where the keyring's passphrase is read from unless a file is given.
const KEYRING_ENV: &str = "THOR_KEYRING_PASSPHRASE";

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
    /// The web, for the fetch tool: one HTTPS client with the address checks.
    pub web: Box<dyn Web>,
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
    /// The engine serving `model`, when one of the other engines serves it.
    #[must_use]
    fn engine_for(&self, model: Option<&str>) -> Option<&Engine> {
        self.engines
            .iter()
            .find(|engine| Some(engine.model.as_str()) == model)
    }

    /// The server for requests naming `model`: the engine serving it, else
    /// llama-server.
    #[must_use]
    pub fn serving(&self, model: Option<&str>) -> &Endpoint {
        self.engine_for(model)
            .map_or(&self.model, |engine| &engine.endpoint)
    }

    /// Whether one of the other engines serves `model`, rather than llama-server.
    ///
    /// Such an engine refuses a body field it does not know, so the request is
    /// cleaned before it is sent there; see `chat::answer`.
    #[must_use]
    pub fn serves_engine(&self, model: Option<&str>) -> bool {
        self.engine_for(model).is_some()
    }

    /// The server for the model a JSON request `body` names.
    #[must_use]
    pub fn serving_body(&self, body: &[u8]) -> &Endpoint {
        self.serving(named(body).as_deref())
    }

    /// Whether one of the other engines serves the model a JSON request `body`
    /// names, rather than llama-server. An engine is stricter about the body, so
    /// the request is rewritten before it is sent there.
    #[must_use]
    pub fn serves_engine_body(&self, body: &[u8]) -> bool {
        self.serves_engine(named(body).as_deref())
    }
}

/// The model a JSON request `body` names, when it names one.
fn named(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Named {
        model: Option<String>,
    }
    let named: Option<Named> = serde_json::from_slice(body).ok();
    named.and_then(|named| named.model)
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
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
                model: Endpoint::new(options.model, authorization),
                engines: options
                    .engines
                    .into_iter()
                    .map(|(model, address)| Engine {
                        model,
                        endpoint: Endpoint::new(address, None),
                    })
                    .collect(),
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

/// The options as given, before the key file is read.
struct Options {
    listen: String,
    model: String,
    engines: Vec<(String, String)>,
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
            model: DEFAULT_MODEL.to_string(),
            engines: Vec::new(),
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
            "--model" => self.model = value,
            "--engine" => self.engines.push(engine(&value)?),
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
fn engine(value: &str) -> Outcome<(String, String)> {
    match value.split_once('=') {
        Some((model, address)) if !model.is_empty() && !address.is_empty() => {
            Ok((model.to_string(), address.to_string()))
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
mod tests {
    use super::*;

    const PASS: &str = "keyring passphrase";

    struct Folder {
        path: PathBuf,
    }

    impl Folder {
        fn new(name: &str) -> Result<Self, AgentError> {
            let path = std::env::temp_dir()
                .join(format!("thor-agent-config-{name}-{}", std::process::id()));
            fs::create_dir_all(&path)
                .map_err(|error| AgentError::Config(format!("{}: {error}", path.display())))?;
            Ok(Self { path })
        }

        fn keyring(&self) -> PathBuf {
            self.path.join("keyring")
        }

        fn passphrase(&self) -> Result<PathBuf, AgentError> {
            let path = self.path.join("passphrase");
            fs::write(&path, format!("{PASS}\n"))
                .map_err(|error| AgentError::Config(format!("{}: {error}", path.display())))?;
            Ok(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    fn with_key(key: Option<&str>) -> Outcome<Config> {
        let mut config = Config::from_args(args(&["--key-file", "/nonexistent"]))?;
        config.key = key.map(ToString::to_string);
        Ok(config)
    }

    fn with_people(folder: &Folder) -> Outcome<(Config, Keyring)> {
        let mut keyring = Keyring::create(folder.keyring(), PASS)?;
        keyring.request("Ada", "ada@example.com")?;
        keyring.approve("ada@example.com")?;
        let passphrase = folder.passphrase()?.display().to_string();
        let keyring_path = folder.keyring().display().to_string();
        let arguments = args(&[
            "--key-file",
            "/nonexistent",
            "--keyring",
            &keyring_path,
            "--keyring-passphrase-file",
            &passphrase,
        ]);
        let config = Config::from_args(arguments)?;
        Ok((config, keyring))
    }

    #[test]
    fn defaults_point_at_localhost() -> Outcome {
        let config = Config::from_args(args(&["--key-file", "/nonexistent"]))?;
        assert_eq!(config.listen, "127.0.0.1:8080");
        assert_eq!(config.upstreams.model.address(), "127.0.0.1:8079");
        assert_eq!(config.upstreams.search.address(), "127.0.0.1:8888");
        assert_eq!(config.key, None);
        assert!(config.people.is_none());
        Ok(())
    }

    #[test]
    fn options_override_defaults() -> Outcome {
        let arguments = args(&[
            "--listen",
            "0.0.0.0:9000",
            "--model",
            "m:1",
            "--search",
            "s:2",
            "--web",
            "/srv",
            "--key-file",
            "/none",
        ]);
        let config = Config::from_args(arguments)?;
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
        assert_eq!(
            upstreams.serving(Some("nemotron")).address(),
            "127.0.0.1:8079"
        );
        assert_eq!(upstreams.serving(None).address(), "127.0.0.1:8079");
        assert_eq!(
            upstreams.serving_body(br#"{"model":"qwen"}"#).address(),
            "e:1"
        );
        assert_eq!(
            upstreams.serving_body(br#"{"messages":[]}"#).address(),
            "127.0.0.1:8079"
        );
        assert_eq!(
            upstreams.serving_body(b"not json").address(),
            "127.0.0.1:8079"
        );
        assert_eq!(upstreams.engines[0].endpoint.authorization(), None);
        Ok(())
    }

    #[test]
    fn rejects_an_engine_without_a_model_or_address() {
        for value in ["qwen", "=e:1", "qwen="] {
            let refused = Config::from_args(args(&["--engine", value]));
            assert!(
                refused.is_err_and(|error| error.to_string().ends_with("expected MODEL=HOST:PORT")),
                "{value}"
            );
        }
    }

    #[test]
    fn rejects_unknown_and_incomplete_options() {
        let unknown = Config::from_args(args(&["--port", "1"]));
        let incomplete = Config::from_args(args(&["--listen"]));
        assert!(unknown.is_err_and(|error| error.to_string() == "config: unknown option --port"));
        assert!(
            incomplete.is_err_and(|error| error.to_string() == "config: --listen needs a value")
        );
    }

    #[test]
    fn reads_the_key_file_trimmed() -> Outcome {
        let path = std::env::temp_dir().join(format!("thor-key-{}", std::process::id()));
        fs::write(&path, "  secret\n")?;
        let config = Config::from_args(args(&["--key-file", &path.to_string_lossy()]));
        fs::remove_file(&path)?;
        let config = config?;
        assert_eq!(config.key.as_deref(), Some("secret"));
        assert_eq!(
            config.upstreams.model.authorization(),
            Some("Bearer secret")
        );
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

    #[test]
    fn a_keyring_lets_each_person_in() -> Outcome {
        let folder = Folder::new("people")?;
        let (config, keyring) = with_people(&folder)?;
        let ada = keyring
            .find("ada@example.com")
            .and_then(|person| person.key.clone());
        let ada = ada.ok_or_else(|| AgentError::Config("ada has no key".to_string()))?;
        assert!(config.admits(Some(&format!("Bearer {ada}"))));
        assert!(!config.admits(Some("Bearer not-a-key")));
        assert!(!config.admits(None));
        assert!(!config.admits(Some("nonsense")));

        config.request_access("Bob", "bob@example.com")?;
        let mut reopened = Keyring::open(folder.keyring(), PASS)?;
        assert!(reopened.find("bob@example.com").is_some());

        let bob = reopened.approve("bob@example.com")?;
        assert!(config.admits(Some(&format!("Bearer {bob}"))));
        Ok(())
    }

    #[test]
    fn a_keyring_that_goes_away_is_reported_and_keeps_the_old_keys() -> Outcome {
        let folder = Folder::new("gone")?;
        let (config, _keyring) = with_people(&folder)?;
        fs::remove_file(folder.keyring())?;
        assert!(!config.admits(Some("Bearer anything")));
        Ok(())
    }

    #[test]
    fn keyring_settings_have_to_be_complete() -> Outcome {
        let folder = Folder::new("settings")?;
        let missing = folder.path.join("nowhere").display().to_string();
        let name = folder.keyring().display().to_string();
        let passphrase = folder.passphrase()?.display().to_string();
        assert!(matches!(
            Config::from_args(args(&[
                "--key-file",
                "/none",
                "--keyring",
                &missing,
                "--keyring-passphrase-file",
                &passphrase,
            ])),
            Err(AgentError::Keyring(_))
        ));
        assert!(matches!(
            Config::from_args(args(&["--key-file", "/none", "--keyring", &name])),
            Err(AgentError::Config(_))
        ));

        let empty = folder.path.join("empty");
        fs::write(&empty, "\n")
            .map_err(|error| AgentError::Config(format!("{}: {error}", empty.display())))?;
        let empty = empty.display().to_string();
        let options = Options {
            keyring: Some(PathBuf::from(&name)),
            keyring_passphrase_file: Some(PathBuf::from(&empty)),
            ..Options::default()
        };
        assert!(matches!(
            keyring_passphrase(&options, Some("from-the-env".to_string())),
            Err(AgentError::Config(_))
        ));
        let from_env = Options {
            keyring: Some(PathBuf::from(&name)),
            ..Options::default()
        };
        assert!(keyring_passphrase(&from_env, Some("from-the-env".to_string())).is_ok());
        assert!(matches!(
            keyring_passphrase(&from_env, Some(String::new())),
            Err(AgentError::Config(_))
        ));
        assert!(matches!(
            keyring_passphrase(
                &Options {
                    keyring: Some(PathBuf::from(&name)),
                    keyring_passphrase_file: Some(PathBuf::from(&missing)),
                    ..Options::default()
                },
                None,
            ),
            Err(AgentError::Config(_))
        ));
        Ok(())
    }

    #[test]
    fn registration_is_closed_without_a_keyring() -> Outcome {
        let config = with_key(None)?;
        assert!(matches!(
            config.request_access("Ada", "ada@example.com"),
            Err(AgentError::BadRequest(_))
        ));
        Ok(())
    }
}
