use super::*;

struct Folder {
    path: PathBuf,
}

impl Folder {
    fn new(name: &str) -> Result<Self, KeyringError> {
        let path = std::env::temp_dir().join(format!("thor-keyring-{name}-{}", std::process::id()));
        fs::create_dir_all(&path).map_err(KeyringError::io(&path))?;

        Ok(Self { path })
    }

    fn keyring(&self) -> PathBuf {
        self.path.join("keyring")
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

const PASS: &str = "correct horse battery staple";

fn made(name: &str) -> Result<(Folder, Keyring), KeyringError> {
    let folder = Folder::new(name)?;
    let keyring = Keyring::create(folder.keyring(), PASS)?;

    Ok((folder, keyring))
}

#[test]
fn a_new_keyring_is_empty_and_reopens_with_the_same_passphrase() -> Outcome<()> {
    let (folder, keyring) = made("round")?;

    assert!(keyring.records().is_empty());

    let reopened = Keyring::open(folder.keyring(), PASS)?;

    assert!(reopened.records().is_empty());

    Ok(())
}

#[test]
fn creating_twice_is_refused() -> Outcome<()> {
    let folder = Folder::new("twice")?;
    Keyring::create(folder.keyring(), PASS)?;

    assert!(matches!(
        Keyring::create(folder.keyring(), PASS),
        Err(KeyringError::Exists { .. })
    ));
    assert!(matches!(
        Keyring::create(folder.keyring(), ""),
        Err(KeyringError::EmptyPassphrase)
    ));
    assert!(matches!(
        Keyring::open(folder.keyring(), ""),
        Err(KeyringError::EmptyPassphrase)
    ));

    Ok(())
}

#[test]
fn a_wrong_passphrase_does_not_open_the_keyring() -> Outcome<()> {
    let (folder, _) = made("wrong")?;

    assert!(matches!(
        Keyring::open(folder.keyring(), "not the passphrase"),
        Err(KeyringError::Sealed)
    ));
    assert!(matches!(
        Keyring::open(folder.path.join("missing"), PASS),
        Err(KeyringError::Io { .. })
    ));

    Ok(())
}

#[test]
fn what_is_not_a_keyring_is_refused() -> Outcome<()> {
    let folder = Folder::new("format")?;
    let path = folder.keyring();
    fs::write(&path, b"too short").map_err(KeyringError::io(&path))?;

    assert!(matches!(
        Keyring::open(&path, PASS),
        Err(KeyringError::Format { .. })
    ));

    let mut wrong_magic = vec![0u8; MIN_LEN + 8];
    wrong_magic.extend_from_slice(MAGIC);
    fs::write(&path, &wrong_magic).map_err(KeyringError::io(&path))?;

    assert!(matches!(
        Keyring::open(&path, PASS),
        Err(KeyringError::Format { .. })
    ));

    let mut truncated = MAGIC.to_vec();
    truncated.extend_from_slice(&[0u8; SALT_LEN]);
    fs::write(&path, &truncated).map_err(KeyringError::io(&path))?;

    assert!(matches!(
        Keyring::open(&path, PASS),
        Err(KeyringError::Format { .. })
    ));

    let mut short_salt = MAGIC.to_vec();
    short_salt.extend_from_slice(&[0u8; 5]);
    fs::write(&path, &short_salt).map_err(KeyringError::io(&path))?;

    assert!(matches!(
        Keyring::open(&path, PASS),
        Err(KeyringError::Format { .. })
    ));

    Ok(())
}

#[test]
fn a_sealed_file_that_is_not_json_is_refused() -> Outcome<()> {
    let folder = Folder::new("json")?;
    let path = folder.keyring();
    let salt = [1u8; SALT_LEN];
    let nonce = [2u8; NONCE_LEN];
    let key = crypto::derive_key(PASS.as_bytes(), &salt)?;
    let sealed = crypto::seal(key.as_slice(), &nonce, MAGIC, b"not json")?;
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&salt);
    bytes.extend_from_slice(&nonce);
    bytes.extend_from_slice(&sealed);
    fs::write(&path, &bytes).map_err(KeyringError::io(&path))?;

    assert!(matches!(
        Keyring::open(&path, PASS),
        Err(KeyringError::Json { .. })
    ));

    Ok(())
}

#[test]
fn a_request_is_recorded_once_and_must_look_like_a_person() -> Outcome<()> {
    let (folder, mut keyring) = made("request")?;
    let added = keyring.request("  Ada Lovelace ", "ada@example.com")?;

    assert_eq!(added.name, "Ada Lovelace");
    assert_eq!(added.status, Status::Requested);
    assert_eq!(added.key, None);
    assert!(keyring.active_keys().is_empty());

    let again = keyring.request("Ada", "ada@example.com")?;

    assert_eq!(again.name, "Ada Lovelace");
    assert_eq!(keyring.records().len(), 1);
    assert!(matches!(
        keyring.request("", "ada@example.com"),
        Err(KeyringError::Invalid(_))
    ));
    assert!(matches!(
        keyring.request("Ada", "not-an-address"),
        Err(KeyringError::Invalid(_))
    ));
    assert!(matches!(
        keyring.request("Ada", ""),
        Err(KeyringError::Invalid(_))
    ));

    let reopened = Keyring::open(folder.keyring(), PASS)?;

    assert_eq!(reopened.records().len(), 1);

    let before = keyring.records().first().map(|person| person.created);
    let after = reopened.records().first().map(|person| person.created);

    assert_eq!(before, after);

    Ok(())
}

#[test]
fn approving_mints_a_working_key_and_then_refuses_a_second() -> Outcome<()> {
    let (folder, mut keyring) = made("approve")?;

    assert!(matches!(
        keyring.approve("nobody@example.com"),
        Err(KeyringError::NotFound(_))
    ));

    keyring.request("Ada", "ada@example.com")?;

    let key = keyring.approve("ada@example.com")?;

    assert_eq!(key.len(), KEY_BYTES * 2);
    assert!(key.chars().all(|digit| digit.is_ascii_hexdigit()));
    assert!(keyring.active_keys().contains(&key));
    assert_eq!(
        keyring.find("ada@example.com").map(|person| person.status),
        Some(Status::Active)
    );
    assert_eq!(
        keyring.find(&key).map(|person| person.email.as_str()),
        Some("ada@example.com")
    );
    assert!(matches!(
        keyring.approve("ada@example.com"),
        Err(KeyringError::AlreadyActive(_))
    ));

    let reopened = Keyring::open(folder.keyring(), PASS)?;

    assert_eq!(reopened.active_keys(), [key]);

    Ok(())
}

#[test]
fn revoking_takes_the_key_away_by_email_or_by_key() -> Outcome<()> {
    let (_folder, mut keyring) = made("revoke")?;
    keyring.request("Ada", "ada@example.com")?;

    let ada = keyring.approve("ada@example.com")?;

    keyring.request("Bob", "bob@example.com")?;

    let bob = keyring.approve("bob@example.com")?;
    let revoked = keyring.revoke(&bob)?;

    assert_eq!(revoked.status, Status::Revoked);
    assert_eq!(revoked.key, None);
    assert!(keyring.active_keys().contains(&ada));

    keyring.revoke("ada@example.com")?;

    assert!(keyring.active_keys().is_empty());
    assert!(matches!(
        keyring.revoke("gone@example.com"),
        Err(KeyringError::NotFound(_))
    ));
    assert!(matches!(
        keyring.revoke(&ada),
        Err(KeyringError::NotFound(_))
    ));

    Ok(())
}

#[test]
fn revoking_everything_stops_the_whole_chat_at_once() -> Outcome<()> {
    let (_folder, mut keyring) = made("revoke-all")?;
    keyring.request("Ada", "ada@example.com")?;
    keyring.approve("ada@example.com")?;
    keyring.request("Bob", "bob@example.com")?;
    keyring.approve("bob@example.com")?;
    keyring.request("Cara", "cara@example.com")?;

    assert_eq!(keyring.revoke_all()?, 2);
    assert!(keyring.active_keys().is_empty());
    assert_eq!(keyring.revoke_all()?, 0);
    assert_eq!(
        keyring
            .records()
            .iter()
            .filter(|person| person.status == Status::Revoked)
            .count(),
        2
    );
    assert_eq!(
        keyring.find("cara@example.com").map(|person| person.status),
        Some(Status::Requested)
    );

    Ok(())
}

#[test]
fn a_name_and_an_email_have_to_look_like_them() {
    assert!(valid_name("Ada"));
    assert!(!valid_name("   "));
    assert!(!valid_name(&"a".repeat(MAX_NAME + 1)));
    assert!(!valid_name("a\nb"));
    assert!(valid_email("ada@example.com"));
    assert!(!valid_email("ada@example"));
    assert!(!valid_email("@example.com"));
    assert!(!valid_email("ada@"));
    assert!(!valid_email("ada @example.com"));
    assert!(!valid_email(".ada@.com"));
    assert!(!valid_email("a@b."));
    assert!(!valid_email(&format!(
        "{}@example.com",
        "a".repeat(MAX_EMAIL)
    )));
}

#[test]
fn timestamps_are_utc_and_known() {
    assert_eq!(timestamp(0), "1970-01-01 00:00:00Z");
    assert_eq!(timestamp(1_000_000_000), "2001-09-09 01:46:40Z");
    assert_eq!(timestamp(1_700_000_000), "2023-11-14 22:13:20Z");
    assert_eq!(timestamp(1_759_795_200), "2025-10-07 00:00:00Z");
    assert!(now() > 1_700_000_000);
}
