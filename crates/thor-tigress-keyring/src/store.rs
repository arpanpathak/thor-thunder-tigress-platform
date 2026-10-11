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
mod tests;
