use super::*;
use std::{io::Cursor, path::Path};

/// A fresh folder for one test, removed when dropped.
struct Folder(tempfile::TempDir);

impl Folder {
    fn new() -> Result<Self, KeyringError> {
        let folder = tempfile::tempdir().map_err(KeyringError::io("temporary folder"))?;

        Ok(Self(folder))
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    fn keyring(&self) -> String {
        self.path().join("keyring").display().to_string()
    }
}

const PASS: &str = "test passphrase";

/// What one run of the program ended with, printed and asked.
struct Ran {
    outcome: Outcome<()>,
    said: String,
    asked: String,
}

/// Runs `words` with `passphrase` in the environment and `typed` on the input.
fn run_with(words: &[String], passphrase: Option<&str>, typed: &str) -> Ran {
    let mut out = Vec::new();
    let mut prompt = Vec::new();
    let mut source = Cursor::new(typed.as_bytes().to_vec());
    let passphrase = passphrase.map(ToString::to_string);
    let outcome = main_with(words, passphrase, &mut source, &mut prompt, &mut out);

    Ran {
        outcome,
        said: String::from_utf8_lossy(&out).into_owned(),
        asked: String::from_utf8_lossy(&prompt).into_owned(),
    }
}

fn run(arguments: &[&str]) -> (Outcome<()>, String) {
    let words: Vec<String> = arguments.iter().map(ToString::to_string).collect();
    let ran = run_with(&words, Some(PASS), "");

    (ran.outcome, ran.said)
}

/// The words of a command on the keyring in `folder`.
fn on_keyring(folder: &Folder, rest: &[&str]) -> Vec<String> {
    ["--keyring", &folder.keyring()]
        .iter()
        .chain(rest)
        .map(ToString::to_string)
        .collect()
}

/// What a command on the keyring in `folder` prints.
fn said(folder: &Folder, rest: &[&str]) -> Outcome<String> {
    let ran = run_with(&on_keyring(folder, rest), Some(PASS), "");

    ran.outcome.map(|()| ran.said)
}

/// A keyring in which Ada has asked for a key.
fn with_a_request() -> Outcome<Folder> {
    let folder = Folder::new()?;

    said(&folder, &["init"])?;
    said(&folder, &["request", "Ada Lovelace", "ada@example.com"])?;

    Ok(folder)
}

/// The key printed when Ada is approved.
fn approved(folder: &Folder) -> Outcome<String> {
    let printed = said(folder, &["approve", "ada@example.com"])?;

    assert!(printed.contains("key for Ada Lovelace <ada@example.com>:"));

    Ok(printed.lines().nth(1).unwrap_or_default().to_string())
}

#[test]
fn a_new_keyring_is_empty_until_someone_asks() -> Outcome<()> {
    let folder = Folder::new()?;

    assert!(said(&folder, &["init"])?.starts_with("created "));
    assert_eq!(said(&folder, &["requests"])?, "nobody is waiting\n");

    let recorded = said(&folder, &["request", "Ada Lovelace", "ada@example.com"])?;
    let waiting = said(&folder, &["requests"])?;

    assert!(recorded.contains("recorded Ada Lovelace <ada@example.com>"));
    assert!(waiting.starts_with("1 waiting\n"));
    assert!(waiting.contains("Ada Lovelace  <ada@example.com>"));

    Ok(())
}

#[test]
fn an_approved_person_has_a_key_that_is_listed_shown_and_exported() -> Outcome<()> {
    let folder = with_a_request()?;
    let minted = approved(&folder)?;
    let listed = said(&folder, &["keys"])?;

    assert_eq!(minted.len(), store::KEY_BYTES * 2);
    assert!(listed.contains("NAME"));
    assert!(listed.contains("Ada Lovelace"));
    assert!(listed.contains(&format!("{}…", minted.get(..8).unwrap_or_default())));
    assert_eq!(
        said(&folder, &["show", "ada@example.com"])?.trim_end(),
        minted
    );
    assert_eq!(said(&folder, &["export"])?, format!("{minted}\n"));

    Ok(())
}

#[test]
fn a_revoked_key_is_gone_and_its_record_stays() -> Outcome<()> {
    let folder = with_a_request()?;

    approved(&folder)?;

    let revoked = said(&folder, &["revoke", "ada@example.com"])?;
    let shown = said(&folder, &["show", "ada@example.com"]);
    let listed = said(&folder, &["keys"])?;

    assert_eq!(revoked, "revoked Ada Lovelace <ada@example.com>\n");
    assert!(said(&folder, &["export"])?.is_empty());
    assert!(shown.is_err_and(|error| error.to_string().contains("has no key")));
    assert!(listed.contains("revoked"));
    assert!(listed.contains("Ada Lovelace"));

    Ok(())
}

#[test]
fn a_bad_command_line_says_so() {
    let cases: [(&[&str], &str); 7] = [
        (&["--colour", "1"], "unknown option --colour"),
        (&["request"], "request needs a name"),
        (&["request", "a", "b", "c"], "request needs a name"),
        (&["keys", "extra"], "keys takes no arguments"),
        (&["--keyring"], "--keyring needs a value"),
        (&["nonsense"], "unknown command nonsense"),
        (&["approve"], "approve needs one email address or key"),
    ];

    for (arguments, expected) in cases {
        let (outcome, said) = run(arguments);

        assert!(said.is_empty());
        assert!(
            matches!(&outcome, Err(error @ KeyringError::Usage(_)) if error.to_string().contains(expected)),
            "{arguments:?}: {outcome:?}"
        );
    }
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
        let (outcome, said) = run(&arguments);
        outcome?;

        assert!(said.starts_with("usage: thor-tigress-keyring"));
    }

    Ok(())
}

#[test]
fn a_missing_keyring_is_an_error() {
    let (opened, _) = run(&["--keyring", "/nonexistent/keyring", "keys"]);

    assert!(matches!(opened, Err(KeyringError::Io { .. })));
}

/// A keyring made with a passphrase file, and the file.
fn sealed_with_a_file() -> Outcome<(Folder, PathBuf)> {
    let folder = Folder::new()?;
    let secret = folder.path().join("pass");
    fs::write(&secret, "from the file\n").map_err(KeyringError::io(&secret))?;

    let file = secret.display().to_string();
    let init = on_keyring(&folder, &["--passphrase-file", &file, "init"]);

    run_with(&init, None, "").outcome?;

    Ok((folder, secret))
}

#[test]
fn the_passphrase_comes_from_the_environment_or_the_input() -> Outcome<()> {
    let (folder, _) = sealed_with_a_file()?;
    let keys = on_keyring(&folder, &["keys"]);
    let from_environment = run_with(&keys, Some("from the file"), "");
    let typed = run_with(&keys, None, "from the file\n");

    from_environment.outcome?;
    typed.outcome?;

    assert!(from_environment.said.contains("the keyring is empty"));
    assert!(typed.said.contains("the keyring is empty"));
    assert!(typed.asked.starts_with("passphrase for "));

    Ok(())
}

#[test]
fn a_wrong_empty_or_unreadable_passphrase_is_refused() -> Outcome<()> {
    let (folder, _) = sealed_with_a_file()?;
    let keys = on_keyring(&folder, &["keys"]);
    let gone = folder.path().join("gone").display().to_string();
    let from_missing_file = on_keyring(&folder, &["--passphrase-file", &gone, "keys"]);
    let wrong = run_with(&keys, Some("wrong"), "").outcome;
    let nothing_typed = run_with(&keys, None, "").outcome;
    let empty = run_with(&keys, Some(""), "").outcome;
    let unreadable = run_with(&from_missing_file, None, "").outcome;

    assert!(matches!(wrong, Err(KeyringError::Sealed)));
    assert!(matches!(nothing_typed, Err(KeyringError::EmptyPassphrase)));
    assert!(matches!(empty, Err(KeyringError::EmptyPassphrase)));
    assert!(matches!(unreadable, Err(KeyringError::Io { .. })));

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
    let folder = Folder::new()?;

    said(&folder, &["init"])?;

    for (name, email) in [("Ada", "ada@example.com"), ("Bob", "bob@example.com")] {
        said(&folder, &["request", name, email])?;
        said(&folder, &["approve", email])?;
    }

    let stopped = said(&folder, &["revoke-all"])?;
    let exported = said(&folder, &["export"])?;
    let again = said(&folder, &["revoke-all"])?;

    assert_eq!(stopped, "revoked 2 active key(s); the records stay\n");
    assert!(exported.is_empty());
    assert_eq!(again, "revoked 0 active key(s); the records stay\n");

    Ok(())
}
