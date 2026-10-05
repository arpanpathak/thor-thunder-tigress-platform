//! The one error type of the crate.

use std::{fmt, io};

/// Everything that can go wrong while serving a request.
#[derive(Debug)]
pub enum AgentError {
    /// Reading or writing a socket or file failed.
    Io(io::Error),
    /// A body that should be JSON is not.
    Json(serde_json::Error),
    /// The request is malformed or not allowed.
    BadRequest(String),
    /// The model server or the search engine answered with an error.
    Upstream(String),
    /// The command line or a setting is wrong.
    Config(String),
}

impl fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Io(error) => write!(formatter, "i/o: {error}"),
            AgentError::Json(error) => write!(formatter, "json: {error}"),
            AgentError::BadRequest(message) => write!(formatter, "bad request: {message}"),
            AgentError::Upstream(message) => write!(formatter, "upstream: {message}"),
            AgentError::Config(message) => write!(formatter, "config: {message}"),
        }
    }
}

impl std::error::Error for AgentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AgentError::Io(error) => Some(error),
            AgentError::Json(error) => Some(error),
            AgentError::BadRequest(_) | AgentError::Upstream(_) | AgentError::Config(_) => None,
        }
    }
}

impl From<io::Error> for AgentError {
    fn from(error: io::Error) -> Self {
        AgentError::Io(error)
    }
}

impl From<serde_json::Error> for AgentError {
    fn from(error: serde_json::Error) -> Self {
        AgentError::Json(error)
    }
}
