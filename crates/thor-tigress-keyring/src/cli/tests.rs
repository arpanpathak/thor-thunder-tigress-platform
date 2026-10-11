use super::*;
use std::io::Cursor;

struct Folder {
    path: PathBuf,
}

impl Folder {
    fn new(name: &str) -> Result<Self, KeyringError> {
        let path =
            std::env::temp_dir().join(format!("thor-keyring-cli-{name}-{}", std::process::id()));
        fs::create_dir_all(&path).map_err(KeyringError::io(&path))?;

        Ok(Self { path })
    }

    fn keyring(&self) -> String {
        self.path.join("keyring").display().to_string()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

const PASS: &str = "test passphrase";

fn run(arguments: &[&str], input: &str) -> (Outcome<()>, String) {
    let words: Vec<String> = arguments.iter().map(ToString::to_string).collect();
    let mut out = Vec::new();
    let mut prompt = Vec::new();
    let mut source = Cursor::new(input.as_bytes().to_vec());
    let outcome = main_with(
        &words,
        Some(PASS.to_string()),
        &mut source,
        &mut prompt,
        &mut out,
    );

    (outcome, String::from_utf8_lossy(&out).into_owned())
}

fn run_in(folder: &Folder, rest: &[&str]) -> (Outcome<()>, String) {
    let mut words = vec!["--keyring".to_string(), folder.keyring()];
    words.extend(rest.iter().map(ToString::to_string));

    let mut out = Vec::new();
    let mut prompt = Vec::new();
    let mut source = Cursor::new(Vec::new());
    let outcome = main_with(
        &words,
        Some(PASS.to_string()),
        &mut source,
        &mut prompt,
        &mut out,
    );

    (outcome, String::from_utf8_lossy(&out).into_owned())
}

#[test]
fn the_whole_life_of_a_request_is_printed() -> Outcome<()> {
    let folder = Folder::new("life")?;
    let (init, said) = run_in(&folder, &["init"]);
    init?;

    assert!(said.starts_with("created "));

    let (nothing, said) = run_in(&folder, &["requests"]);
    nothing?;

    assert_eq!(said, "nobody is waiting\n");

    let (asked, said) = run_in(&folder, &["request", "Ada Lovelace", "ada@example.com"]);
    asked?;

    assert!(said.contains("recorded Ada Lovelace <ada@example.com>"));

    let (waiting, said) = run_in(&folder, &["requests"]);
    waiting?;

    assert!(said.starts_with("1 waiting\n"));
    assert!(said.contains("Ada Lovelace  <ada@example.com>"));

    let (key, said) = run_in(&folder, &["approve", "ada@example.com"]);
    key?;

    assert!(said.contains("key for Ada Lovelace <ada@example.com>:"));

    let minted = said.lines().nth(1).unwrap_or_default().to_string();

    assert_eq!(minted.len(), store::KEY_BYTES * 2);

    let (all, said) = run_in(&folder, &["keys"]);
    all?;

    assert!(said.contains("NAME"));
    assert!(said.contains("Ada Lovelace"));
    assert!(said.contains(&format!("{}…", minted.get(..8).unwrap_or_default())));

    let (shown, said) = run_in(&folder, &["show", "ada@example.com"]);
    shown?;

    assert_eq!(said.trim_end(), minted);

    let (exported, said) = run_in(&folder, &["export"]);
    exported?;

    assert_eq!(said, format!("{minted}\n"));

    let (revoked, said) = run_in(&folder, &["revoke", "ada@example.com"]);
    revoked?;

    assert_eq!(said, "revoked Ada Lovelace <ada@example.com>\n");

    let (gone, said) = run_in(&folder, &["export"]);
    gone?;

    assert!(said.is_empty());

    let (taken, _) = run_in(&folder, &["show", "ada@example.com"]);

    assert!(
        taken
            .err()
            .is_some_and(|error| error.to_string().contains("has no key"))
    );

    let (still_there, said) = run_in(&folder, &["keys"]);
    still_there?;

    assert!(said.contains("revoked"));
    assert!(said.contains("Ada Lovelace"));

    Ok(())
}

#[test]
fn a_bad_command_line_says_so() {
    let (unknown, said) = run(&["--colour", "1"], "");

    assert!(matches!(unknown, Err(KeyringError::Usage(_))));
    assert!(said.is_empty());

    let (missing, _) = run(&["request"], "");

    assert!(
        missing
            .err()
            .is_some_and(|error| error.to_string().contains("request needs a name"))
    );

    let (extra, _) = run(&["request", "a", "b", "c"], "");

    assert!(matches!(extra, Err(KeyringError::Usage(_))));

    let (empty, _) = run(&["keys", "extra"], "");

    assert!(
        empty
            .err()
            .is_some_and(|error| error.to_string().contains("keys takes no arguments"))
    );

    let (flag, _) = run(&["--keyring"], "");

    assert!(
        flag.err()
            .is_some_and(|error| error.to_string().starts_with("--keyring needs a value"))
    );

    let (command, _) = run(&["nonsense"], "");

    assert!(
        command
            .err()
            .is_some_and(|error| error.to_string().starts_with("unknown command nonsense"))
    );

    let (one, _) = run(&["approve"], "");

    assert!(one.err().is_some_and(|error| {
        error
            .to_string()
            .contains("approve needs one email address or key")
    }));
}

#[test]
fn help_is_printed_without_touching_a_keyring() -> Outcome<()> {
    for arguments in [
        vec![],
        vec!["help"],
        vec!["-h"],
        vec!["--help"],
        vec!["keys", "--help"],
    ] {
        let (outcome, said) = run(&arguments, "");
        outcome?;

        assert!(said.starts_with("usage: thor-tigress-keyring"));
    }

    Ok(())
}

#[test]
fn a_missing_keyring_is_an_error() {
    let (opened, _) = run(&["--keyring", "/nonexistent/keyring", "keys"], "");

    assert!(matches!(opened, Err(KeyringError::Io { .. })));
}

#[test]
fn the_passphrase_comes_from_the_file_the_environment_or_the_input() -> Outcome<()> {
    let folder = Folder::new("passphrase")?;
    let secret = folder.path.join("pass");
    fs::write(&secret, "from the file\n").map_err(KeyringError::io(&secret))?;

    let init = vec![
        "--keyring".to_string(),
        folder.keyring(),
        "--passphrase-file".to_string(),
        secret.display().to_string(),
        "init".to_string(),
    ];
    let mut out = Vec::new();
    let mut prompt = Vec::new();
    let mut source = Cursor::new(Vec::new());
    main_with(&init, None, &mut source, &mut prompt, &mut out)?;

    let keys = vec![
        "--keyring".to_string(),
        folder.keyring(),
        "keys".to_string(),
    ];
    let mut out = Vec::new();
    let mut prompt = Vec::new();
    let mut source = Cursor::new(Vec::new());
    let pass = "from the file".to_string();
    let opened = main_with(&keys, Some(pass), &mut source, &mut prompt, &mut out);

    assert!(opened.is_ok());
    assert!(String::from_utf8_lossy(&out).contains("the keyring is empty"));

    let mut out = Vec::new();
    let mut prompt = Vec::new();
    let mut source = Cursor::new(Vec::new());

    assert!(matches!(
        main_with(
            &keys,
            Some("wrong".to_string()),
            &mut source,
            &mut prompt,
            &mut out
        ),
        Err(KeyringError::Sealed)
    ));

    let mut out = Vec::new();
    let mut prompt = Vec::new();
    let mut source = Cursor::new(b"from the file\n".to_vec());
    main_with(&keys, None, &mut source, &mut prompt, &mut out)?;

    assert!(String::from_utf8_lossy(&out).contains("the keyring is empty"));
    assert!(String::from_utf8_lossy(&prompt).starts_with("passphrase for "));

    let mut out = Vec::new();
    let mut prompt = Vec::new();
    let mut source = Cursor::new(Vec::new());

    assert!(matches!(
        main_with(&keys, None, &mut source, &mut prompt, &mut out),
        Err(KeyringError::EmptyPassphrase)
    ));

    let mut out = Vec::new();
    let mut prompt = Vec::new();
    let mut source = Cursor::new(Vec::new());

    assert!(matches!(
        main_with(
            &keys,
            Some(String::new()),
            &mut source,
            &mut prompt,
            &mut out
        ),
        Err(KeyringError::EmptyPassphrase)
    ));

    let missing = vec![
        "--keyring".to_string(),
        folder.keyring(),
        "--passphrase-file".to_string(),
        folder.path.join("gone").display().to_string(),
        "keys".to_string(),
    ];
    let mut out = Vec::new();
    let mut prompt = Vec::new();
    let mut source = Cursor::new(Vec::new());

    assert!(matches!(
        main_with(&missing, None, &mut source, &mut prompt, &mut out),
        Err(KeyringError::Io { .. })
    ));

    Ok(())
}

#[test]
fn keys_are_masked_and_statuses_have_words() {
    assert_eq!(masked(Some("0123456789abcdef")), "01234567…");
    assert_eq!(masked(Some("short")), "short");
    assert_eq!(masked(None), "—");
    assert_eq!(status_word(Status::Requested), "requested");
    assert_eq!(status_word(Status::Active), "active");
    assert_eq!(status_word(Status::Revoked), "revoked");
}

#[test]
fn the_report_tables_look_right() {
    let people = vec![Person {
        name: "Ada".to_string(),
        email: "ada@example.com".to_string(),
        status: Status::Active,
        key: Some("0123456789abcdef".to_string()),
        created: 1_700_000_000,
    }];
    let table = report_keys(&people);

    assert!(table.starts_with("NAME"));
    assert!(table.contains("Ada"));
    assert!(table.contains("01234567…"));
    assert_eq!(report_requests(&people), "nobody is waiting\n");
}

#[test]
fn revoke_all_stops_everyone_at_once() -> Outcome<()> {
    let folder = Folder::new("revoke-all")?;
    let (init, _) = run_in(&folder, &["init"]);
    init?;

    for (name, email) in [("Ada", "ada@example.com"), ("Bob", "bob@example.com")] {
        let (asked, _) = run_in(&folder, &["request", name, email]);
        asked?;

        let (approved, _) = run_in(&folder, &["approve", email]);
        approved?;
    }

    let (stopped, said) = run_in(&folder, &["revoke-all"]);
    stopped?;

    assert_eq!(said, "revoked 2 active key(s); the records stay\n");

    let (exported, said) = run_in(&folder, &["export"]);
    exported?;

    assert!(said.is_empty());

    let (again, said) = run_in(&folder, &["revoke-all"]);
    again?;

    assert_eq!(said, "revoked 0 active key(s); the records stay\n");

    Ok(())
}
