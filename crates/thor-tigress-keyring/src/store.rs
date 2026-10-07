//! The registry itself: who asked, who may use the chat, and the key each
//! person was given.
//!
//! On disk it is one file: a magic line, a random salt, a random nonce, and the
//! JSON of every record sealed behind the passphrase. Every write uses a fresh
//! nonce and a temporary file that is renamed into place, so a crash leaves the
//! old keyring whole.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    crypto::{self, Key, NONCE_LEN, SALT_LEN},
    error::{KeyringError, Outcome},
};

/// The first bytes of every keyring, and the additional data the seal is bound
/// to. The version is in the name, so a later format is refused, not misread.
pub const MAGIC: &[u8] = b"thor-keyring-1\n";

/// The longest name accepted from the registration form.
pub const MAX_NAME: usize = 80;

/// The longest email address accepted from the registration form.
pub const MAX_EMAIL: usize = 200;

/// Bytes of a minted key, before it is written as hex.
pub const KEY_BYTES: usize = 24;

/// The shortest file this format can be.
const MIN_LEN: usize = MAGIC.len() + SALT_LEN + NONCE_LEN;

/// Where a person stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Asked for access from the registration form; waits for the author.
    Requested,
    /// Has a key that the chat accepts.
    Active,
    /// Had a key; it no longer works.
    Revoked,
}

/// One person, and the key they were given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    /// What they asked to be called.
    pub name: String,
    /// Where the author reaches them.
    pub email: String,
    /// Where they stand.
    pub status: Status,
    /// The key, once one was minted; `None` while requested or after a revoke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// When the record was made, in seconds since 1970.
    pub created: u64,
}

/// The open registry: every record, the salt, and the key that seals it.
pub struct Keyring {
    path: PathBuf,
    salt: Vec<u8>,
    key: Key,
    people: Vec<Person>,
}

impl Keyring {
    /// Starts an empty keyring at `path`, sealed with `passphrase`.
    ///
    /// # Errors
    ///
    /// [`KeyringError::Exists`] when the file is already there,
    /// [`KeyringError::EmptyPassphrase`] for an empty passphrase, and the I/O,
    /// randomness and sealing errors of [`Keyring::save`].
    pub fn create(path: impl Into<PathBuf>, passphrase: &str) -> Outcome<Self> {
        let path = path.into();
        if passphrase.is_empty() {
            return Err(KeyringError::EmptyPassphrase);
        }
        if path.exists() {
            return Err(KeyringError::Exists { path });
        }
        let mut salt = vec![0u8; SALT_LEN];
        crypto::random(&mut salt)?;
        let key = crypto::derive_key(passphrase.as_bytes(), &salt)?;
        let keyring = Keyring {
            path,
            salt,
            key,
            people: Vec::new(),
        };
        keyring.save()?;
        Ok(keyring)
    }

    /// Opens the keyring at `path` with `passphrase`.
    ///
    /// # Errors
    ///
    /// [`KeyringError::Format`] when the file is not a keyring of this version,
    /// [`KeyringError::Sealed`] when the passphrase is wrong or the file was
    /// changed, [`KeyringError::Json`] when the records do not parse, and the
    /// I/O and key-stretching errors of reading the file.
    pub fn open(path: impl Into<PathBuf>, passphrase: &str) -> Outcome<Self> {
        let path = path.into();
        if passphrase.is_empty() {
            return Err(KeyringError::EmptyPassphrase);
        }
        let bytes = fs::read(&path).map_err(KeyringError::io(&path))?;
        let refused = || KeyringError::Format {
            path: path.clone(),
            message: "not a keyring of this version".to_string(),
        };
        let (magic, rest) = bytes.split_at_checked(MAGIC.len()).ok_or_else(refused)?;
        if magic != MAGIC {
            return Err(refused());
        }
        let (salt, rest) = rest.split_at_checked(SALT_LEN).ok_or_else(refused)?;
        let (nonce, ciphertext) = rest.split_at_checked(NONCE_LEN).ok_or_else(refused)?;
        let key = crypto::derive_key(passphrase.as_bytes(), salt)?;
        let plaintext = crypto::open(key.as_slice(), nonce, MAGIC, ciphertext)?;
        let people =
            serde_json::from_slice(&plaintext).map_err(|source| json_error(&path, source))?;
        Ok(Keyring {
            path,
            salt: salt.to_vec(),
            key,
            people,
        })
    }

    /// Writes every record back, sealed with a fresh nonce, through a
    /// temporary file so a crash cannot half-write the keyring.
    ///
    /// # Errors
    ///
    /// I/O errors from the temporary file or the rename, and the randomness and
    /// sealing errors of [`crypto::seal`].
    pub fn save(&self) -> Outcome<()> {
        let plaintext =
            serde_json::to_vec(&self.people).map_err(|source| json_error(&self.path, source))?;
        let mut nonce = [0u8; NONCE_LEN];
        crypto::random(&mut nonce)?;
        let sealed = crypto::seal(self.key.as_slice(), &nonce, MAGIC, &plaintext)?;
        let mut bytes = Vec::with_capacity(MIN_LEN + sealed.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.salt);
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&sealed);
        let temporary = self.path.with_extension("new");
        fs::write(&temporary, &bytes).map_err(KeyringError::io(&temporary))?;
        restrict(&temporary)?;
        fs::rename(&temporary, &self.path).map_err(KeyringError::io(&self.path))
    }

    /// Where the keyring lives.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every record, in the order it was made.
    #[must_use]
    pub fn records(&self) -> &[Person] {
        &self.people
    }

    /// The key of every person the chat should let in.
    #[must_use]
    pub fn active_keys(&self) -> Vec<String> {
        self.people
            .iter()
            .filter(|person| person.status == Status::Active)
            .filter_map(|person| person.key.clone())
            .collect()
    }

    /// The record for an email address or a key.
    #[must_use]
    pub fn find(&self, who: &str) -> Option<&Person> {
        self.people
            .iter()
            .find(|person| person.email == who || person.key.as_deref() == Some(who))
    }

    /// Records that someone asked for access. Asking twice with the same email
    /// changes nothing.
    ///
    /// # Errors
    ///
    /// [`KeyringError::Invalid`] for a name or email the form should not have
    /// accepted, and the errors of [`Keyring::save`].
    pub fn request(&mut self, name: &str, email: &str) -> Outcome<Person> {
        let name = name.trim();
        let email = email.trim();
        if !valid_name(name) {
            return Err(KeyringError::Invalid(
                "a name of up to 80 characters is needed".to_string(),
            ));
        }
        if !valid_email(email) {
            return Err(KeyringError::Invalid(
                "a working email address is needed".to_string(),
            ));
        }
        if let Some(existing) = self.people.iter().find(|person| person.email == email) {
            return Ok(existing.clone());
        }
        let person = Person {
            name: name.to_string(),
            email: email.to_string(),
            status: Status::Requested,
            key: None,
            created: now(),
        };
        self.people.push(person.clone());
        self.save()?;
        Ok(person)
    }

    /// Gives the person with `email` a new key, and records it as active.
    ///
    /// # Errors
    ///
    /// [`KeyringError::NotFound`] when nobody asked with that address,
    /// [`KeyringError::AlreadyActive`] when they already have a key, and the
    /// randomness and saving errors of minting one.
    pub fn approve(&mut self, email: &str) -> Outcome<String> {
        let who = email.trim();
        let person = self
            .people
            .iter_mut()
            .find(|person| person.email == who)
            .ok_or_else(|| KeyringError::NotFound(who.to_string()))?;
        if person.status == Status::Active {
            return Err(KeyringError::AlreadyActive(who.to_string()));
        }
        let key = mint()?;
        person.status = Status::Active;
        person.key = Some(key.clone());
        person.created = now();
        self.save()?;
        Ok(key)
    }

    /// Takes the key away from an email address or a key. Revoking someone who
    /// is already revoked is an error, so a typo cannot look like success.
    ///
    /// # Errors
    ///
    /// [`KeyringError::NotFound`] when no record matches, and the errors of
    /// [`Keyring::save`].
    pub fn revoke(&mut self, who: &str) -> Outcome<Person> {
        let who = who.trim();
        let person = self
            .people
            .iter_mut()
            .find(|person| person.email == who || person.key.as_deref() == Some(who))
            .ok_or_else(|| KeyringError::NotFound(who.to_string()))?;
        person.status = Status::Revoked;
        person.key = None;
        let revoked = person.clone();
        self.save()?;
        Ok(revoked)
    }

    /// Takes every active key away at once, for a leak or for closing the chat.
    /// The records stay, so the same people can be approved again later.
    ///
    /// # Errors
    ///
    /// The errors of [`Keyring::save`].
    pub fn revoke_all(&mut self) -> Outcome<usize> {
        let revoked = self
            .people
            .iter_mut()
            .filter(|person| person.status == Status::Active)
            .map(|person| {
                person.status = Status::Revoked;
                person.key = None;
            })
            .count();
        self.save()?;
        Ok(revoked)
    }
}

/// Whether a name from the form is usable.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.chars().count() <= MAX_NAME && !name.chars().any(char::is_control)
}

/// Whether an email address from the form is usable. It only has to reach a
/// person; the author checks it before sending a key.
#[must_use]
pub fn valid_email(email: &str) -> bool {
    let email = email.trim();
    if email.is_empty() || email.len() > MAX_EMAIL || email.chars().any(char::is_whitespace) {
        return false;
    }
    match email.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.len() >= 3
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        }
        None => false,
    }
}

/// The error for records that do not read or write as JSON.
fn json_error(path: &Path, source: serde_json::Error) -> KeyringError {
    KeyringError::Json {
        path: path.to_path_buf(),
        source,
    }
}

/// Seconds since 1970, for a record's timestamp.
#[must_use]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `seconds` since 1970 as `YYYY-MM-DD HH:MM:SSZ`, in UTC.
#[must_use]
pub fn timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// The calendar date `days` after 1970-01-01, by Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (month_prime + if month_prime < 10 { 3 } else { -9 }) as u32;
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

/// A new key: [`KEY_BYTES`] random bytes, lowercase hex.
fn mint() -> Outcome<String> {
    let mut bytes = [0u8; KEY_BYTES];
    crypto::random(&mut bytes)?;
    Ok(bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}

/// Makes `path` readable by its owner alone, when the system has permissions.
#[cfg(unix)]
fn restrict(path: &Path) -> Outcome<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(KeyringError::io(path))
}

/// Nothing to do where file permissions are not a concept.
#[cfg(not(unix))]
fn restrict(_path: &Path) -> Outcome<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder {
        path: PathBuf,
    }

    impl Folder {
        fn new(name: &str) -> Result<Self, KeyringError> {
            let path =
                std::env::temp_dir().join(format!("thor-keyring-{name}-{}", std::process::id()));
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
}
