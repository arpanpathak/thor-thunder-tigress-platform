//! The one error type of the crate.

use std::{fmt, io, path::PathBuf};

/// A keyring result, before it is returned.
pub type Outcome<T> = Result<T, KeyringError>;

/// Everything that can go wrong while reading, writing or changing the
/// registry.
#[derive(Debug)]
pub enum KeyringError {
    /// A file could not be read or written.
    Io {
        /// The file involved.
        path: PathBuf,
        /// What the operating system reported.
        source: io::Error,
    },
    /// The file exists but is not a keyring this version can read.
    Format {
        /// The file involved.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
    /// The file did not open: the passphrase is wrong, or the file was changed
    /// outside the tool.
    Sealed,
    /// The decrypted contents are not the JSON this version writes.
    Json {
        /// The file involved.
        path: PathBuf,
        /// What the parser reported.
        source: serde_json::Error,
    },
    /// The passphrase could not be turned into a key.
    Kdf(argon2::Error),
    /// The operating system refused to give random bytes.
    Random(getrandom::Error),
    /// A keyring already exists at the given path.
    Exists {
        /// The file involved.
        path: PathBuf,
    },
    /// No record matches what was asked for.
    NotFound(String),
    /// The person already has a working key.
    AlreadyActive(String),
    /// A name, an email address or a request is not acceptable.
    Invalid(String),
    /// The passphrase is empty.
    EmptyPassphrase,
    /// A key handed to the cipher is not the 32 bytes it must be.
    KeyLength,
    /// A nonce handed to the cipher is not the 24 bytes it must be.
    NonceLength,
    /// The command line was not understood.
    Usage(String),
}

impl KeyringError {
    /// A closure that wraps an I/O error with the path it happened on.
    pub fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> KeyringError {
        let path = path.into();
        move |source| KeyringError::Io { path, source }
    }
}

impl fmt::Display for KeyringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyringError::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            KeyringError::Format { path, message } => {
                write!(formatter, "{}: {message}", path.display())
            }
            KeyringError::Sealed => {
                formatter.write_str("wrong passphrase, or the file was changed")
            }
            KeyringError::Json { path, source } => {
                write!(formatter, "{}: {source}", path.display())
            }
            KeyringError::Kdf(error) => write!(formatter, "passphrase key: {error}"),
            KeyringError::Random(error) => write!(formatter, "randomness: {error}"),
            KeyringError::Exists { path } => {
                write!(formatter, "{}: already exists", path.display())
            }
            KeyringError::NotFound(who) => write!(formatter, "no record for {who}"),
            KeyringError::AlreadyActive(who) => write!(formatter, "{who} already has a key"),
            KeyringError::Invalid(message) | KeyringError::Usage(message) => {
                formatter.write_str(message)
            }
            KeyringError::EmptyPassphrase => formatter.write_str("the passphrase is empty"),
            KeyringError::KeyLength => formatter.write_str("the key must be 32 bytes"),
            KeyringError::NonceLength => formatter.write_str("the nonce must be 24 bytes"),
        }
    }
}

impl std::error::Error for KeyringError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            KeyringError::Io { source, .. } => Some(source),
            KeyringError::Json { source, .. } => Some(source),
            KeyringError::Kdf(error) => Some(error),
            KeyringError::Random(error) => Some(error),
            KeyringError::Format { .. }
            | KeyringError::Sealed
            | KeyringError::Exists { .. }
            | KeyringError::NotFound(_)
            | KeyringError::AlreadyActive(_)
            | KeyringError::Invalid(_)
            | KeyringError::EmptyPassphrase
            | KeyringError::KeyLength
            | KeyringError::NonceLength
            | KeyringError::Usage(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_error() -> Vec<KeyringError> {
        vec![
            KeyringError::io("keyring")(io::Error::other("disk full")),
            KeyringError::Format {
                path: "keyring".into(),
                message: "not a keyring".to_string(),
            },
            KeyringError::Sealed,
            serde_json::from_str::<u8>("x")
                .err()
                .map(|source| KeyringError::Json {
                    path: "keyring".into(),
                    source,
                })
                .unwrap_or(KeyringError::KeyLength),
            KeyringError::Kdf(argon2::Error::MemoryTooLittle),
            KeyringError::Random(getrandom::Error::UNEXPECTED),
            KeyringError::Exists {
                path: "keyring".into(),
            },
            KeyringError::NotFound("ada@example.com".to_string()),
            KeyringError::AlreadyActive("ada@example.com".to_string()),
            KeyringError::Invalid("a name is needed".to_string()),
            KeyringError::EmptyPassphrase,
            KeyringError::KeyLength,
            KeyringError::NonceLength,
            KeyringError::Usage("usage: thor-tigress-keyring".to_string()),
        ]
    }

    #[test]
    fn every_error_says_what_went_wrong() {
        let said: Vec<String> = every_error().iter().map(ToString::to_string).collect();
        assert_eq!(said[0], "keyring: disk full");
        assert_eq!(said[1], "keyring: not a keyring");
        assert_eq!(said[2], "wrong passphrase, or the file was changed");
        assert!(said[3].starts_with("keyring: "));
        assert!(said[4].starts_with("passphrase key: "));
        assert!(said[5].starts_with("randomness: "));
        assert_eq!(said[6], "keyring: already exists");
        assert_eq!(said[7], "no record for ada@example.com");
        assert_eq!(said[8], "ada@example.com already has a key");
        assert_eq!(said[9], "a name is needed");
        assert_eq!(said[10], "the passphrase is empty");
        assert_eq!(said[11], "the key must be 32 bytes");
        assert_eq!(said[12], "the nonce must be 24 bytes");
        assert_eq!(said[13], "usage: thor-tigress-keyring");
    }

    #[test]
    fn only_the_wrapped_errors_have_a_source() {
        let sources: Vec<bool> = every_error()
            .iter()
            .map(|error| std::error::Error::source(error).is_some())
            .collect();
        assert_eq!(
            sources,
            [
                true, false, false, true, true, true, false, false, false, false, false, false,
                false, false
            ]
        );
    }
}
