//! The one error type of the crate.

use std::io;

use thiserror::Error;

/// The result of anything in this crate that can fail; `Outcome` alone means
/// "done, or an error".
pub type Outcome<T = ()> = Result<T, AgentError>;

/// Everything that can go wrong while serving a request.
#[derive(Debug, Error)]
pub enum AgentError {
    /// Reading or writing a socket or file failed.
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    /// A body that should be JSON is not.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// The request is malformed or not allowed.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// The model server or the search engine answered with an error.
    #[error("upstream: {0}")]
    Upstream(String),
    /// Downloading a page, or turning it into text, failed.
    #[error("fetch: {0}")]
    Fetch(String),
    /// A page was not read because a security rule refused it.
    #[error("refused: {0}")]
    Refused(String),
    /// The encrypted keyring could not be read or written.
    #[error("keyring: {0}")]
    Keyring(String),
    /// The command line or a setting is wrong.
    #[error("config: {0}")]
    Config(String),
}

impl AgentError {
    /// A `BadRequest` with `message`.
    pub fn bad_request(message: impl Into<String>) -> Self {
        AgentError::BadRequest(message.into())
    }
}

impl From<thor_tigress_keyring::error::KeyringError> for AgentError {
    fn from(error: thor_tigress_keyring::error::KeyringError) -> Self {
        match error {
            thor_tigress_keyring::error::KeyringError::Invalid(message) => {
                AgentError::BadRequest(message)
            }
            other => AgentError::Keyring(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_error_keeps_its_source() {
        use std::error::Error as _;

        let error = serde_json::from_str::<u8>("x")
            .map_err(AgentError::from)
            .err();

        assert!(error.as_ref().and_then(|error| error.source()).is_some());
        assert!(
            AgentError::from(std::io::Error::other("x"))
                .source()
                .is_some()
        );
    }
    use std::error::Error as _;

    #[test]
    fn every_variant_names_its_kind() {
        let errors = [
            AgentError::from(io::Error::other("disk")),
            AgentError::bad_request("no body"),
            AgentError::Upstream("502".to_string()),
            AgentError::Fetch("page".to_string()),
            AgentError::Refused("loopback".to_string()),
            AgentError::Keyring("wrong passphrase".to_string()),
            AgentError::Config("--listen needs a value".to_string()),
        ];
        let shown: Vec<String> = errors.iter().map(ToString::to_string).collect();

        assert_eq!(
            shown,
            [
                "i/o: disk",
                "bad request: no body",
                "upstream: 502",
                "fetch: page",
                "refused: loopback",
                "keyring: wrong passphrase",
                "config: --listen needs a value"
            ]
        );
    }

    #[test]
    fn wrapped_errors_keep_their_source() {
        let json: Outcome<serde_json::Value> = serde_json::from_str("{").map_err(AgentError::from);

        assert!(json.is_err_and(
            |error| error.to_string().starts_with("json: ") && error.source().is_some()
        ));
        assert!(AgentError::bad_request("x").source().is_none());
    }
}
