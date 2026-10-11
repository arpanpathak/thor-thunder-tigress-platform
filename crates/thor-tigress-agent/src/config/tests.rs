use super::*;
use crate::testing::{FakeServer, json_response};

const PASS: &str = "keyring passphrase";

struct Folder {
    path: PathBuf,
}

impl Folder {
    fn new(name: &str) -> Result<Self, AgentError> {
        let path =
            std::env::temp_dir().join(format!("thor-agent-config-{name}-{}", std::process::id()));
        fs::create_dir_all(&path)?;

        Ok(Self { path })
    }

    fn keyring(&self) -> PathBuf {
        self.path.join("keyring")
    }

    fn passphrase(&self) -> Result<PathBuf, AgentError> {
        let path = self.path.join("passphrase");
        fs::write(&path, format!("{PASS}\n"))?;

        Ok(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// `list`, behind the one engine every server needs.
fn args(list: &[&str]) -> Vec<String> {
    ["--engine", "nemotron=127.0.0.1:1"]
        .iter()
        .chain(list)
        .map(ToString::to_string)
        .collect()
}

/// `servers`, with a key file that isn't there.
fn without_a_key(servers: &[&str]) -> Vec<String> {
    servers
        .iter()
        .chain(&["--key-file", "/none"])
        .map(ToString::to_string)
        .collect()
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
    assert_eq!(config.upstreams.default.endpoint().address(), "127.0.0.1:1");
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
        "--search",
        "s:2",
        "--web",
        "/srv",
        "--key-file",
        "/none",
    ]);
    let config = Config::from_args(arguments)?;

    assert_eq!(config.listen, "0.0.0.0:9000");
    assert_eq!(config.upstreams.search.address(), "s:2");
    assert_eq!(config.web, PathBuf::from("/srv"));

    Ok(())
}

#[test]
fn a_named_model_goes_to_its_engine_and_any_other_name_to_the_first() -> Outcome {
    let config = Config::from_args(args(&["--engine", "qwen=e:1", "--key-file", "/none"]))?;
    let upstreams = &config.upstreams;
    let served = |model| upstreams.serving(model).endpoint().address();

    assert_eq!(served(Some("qwen")), "e:1");
    assert_eq!(served(Some("nemotron")), "127.0.0.1:1");
    assert_eq!(served(Some("claude-sonnet-5-5")), "127.0.0.1:1");
    assert_eq!(served(None), "127.0.0.1:1");
    assert!(
        upstreams
            .engines()
            .all(|engine| engine.endpoint().authorization().is_none())
    );

    Ok(())
}

#[test]
fn with_no_server_named_llama_server_on_its_usual_port_answers() -> Outcome {
    let config = Config::from_args(["--key-file".to_string(), "/none".to_string()])?;
    let default = &config.upstreams.default;

    assert!(matches!(default, Engine::Llama { .. }));
    assert_eq!(default.endpoint().address(), "127.0.0.1:8079");
    assert_eq!(default.label(), "llama-server");
    assert!(config.upstreams.others.is_empty());

    Ok(())
}

/// llama-server answers for what it has loaded, under an id or an alias;
/// a model it only has on disk is nobody's, so the first server gets it.
#[test]
fn llama_server_gets_the_models_it_has_loaded() -> Outcome {
    let preset = r#"{"data":[
        {"id":"nano","aliases":["nemotron"],"status":{"value":"loaded"}},
        {"id":"on-disk","status":{"value":"unloaded"}}]}"#;
    let llama = FakeServer::start(vec![json_response(preset); 3])?;
    let address = llama.address();
    let servers = ["--engine", "qwen=e:1", "--model", &address];
    let config = Config::from_args(without_a_key(&servers))?;

    let served = ["nemotron", "nano", "on-disk", "qwen"]
        .map(|model| config.upstreams.serving(Some(model)).label());

    assert_eq!(served, ["llama-server", "llama-server", "qwen", "qwen"]);
    assert_eq!(config.upstreams.serving(None).label(), "qwen");

    Ok(())
}

#[test]
fn the_first_server_named_is_the_default() -> Outcome {
    let servers = ["--model", "l:1", "--engine", "qwen=e:1"];
    let config = Config::from_args(without_a_key(&servers))?;

    assert_eq!(config.upstreams.default.label(), "llama-server");
    assert_eq!(config.upstreams.others[0].label(), "qwen");

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
    assert!(incomplete.is_err_and(|error| error.to_string() == "config: --listen needs a value"));
}

#[test]
fn reads_the_key_file_trimmed() -> Outcome {
    let path = std::env::temp_dir().join(format!("thor-key-{}", std::process::id()));
    fs::write(&path, "  secret\n")?;

    let key_file = path.to_string_lossy();
    let config = Config::from_args(args(&["--model", "l:1", "--key-file", &key_file]));

    fs::remove_file(&path)?;

    let config = config?;
    let sent = |engine: &Engine| engine.endpoint().authorization().map(str::to_string);
    let to_llama = sent(&config.upstreams.others[0]);

    assert_eq!(config.key.as_deref(), Some("secret"));
    assert_eq!(config.upstreams.search.authorization(), None);
    assert_eq!(sent(&config.upstreams.default), None);
    assert_eq!(to_llama.as_deref(), Some("Bearer secret"));

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
        .and_then(|person| person.key.clone())
        .unwrap_or_default();

    assert!(!ada.is_empty());
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
    fs::write(&empty, "\n")?;

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
