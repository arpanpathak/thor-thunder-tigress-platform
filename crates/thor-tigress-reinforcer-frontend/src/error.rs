//! The error type for the review tool.

use std::{io, path::Path};

use axum::http::StatusCode;
use thiserror::Error;

/// The result of anything in this crate that can fail.
pub type Outcome<T = ()> = Result<T, ReviewError>;

/// Anything that stops the tool, or that makes one request fail.
#[derive(Debug, Error)]
pub enum ReviewError {
    /// A file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The file involved.
        path: String,
        /// What the operating system reported.
        source: io::Error,
    },
    /// A line of JSONL did not parse.
    #[error("{path}:{line}: {source}")]
    Json {
        /// The file involved.
        path: String,
        /// The one-based line number.
        line: usize,
        /// What the parser reported.
        source: serde_json::Error,
    },
    /// The request asked for something malformed.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// The request asked for something that does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// A thread panicked while it held the flags, so they can't be trusted.
    #[error("the flags are unavailable after a crash; restart the tool")]
    Poisoned,
}

impl ReviewError {
    /// A converter for `map_err` that keeps the path next to the I/O error.
    pub fn io(path: &Path) -> impl FnOnce(io::Error) -> ReviewError {
        let path = path.display().to_string();

        move |source| ReviewError::Io { path, source }
    }

    /// A converter for `map_err` that keeps the path and line next to the parse error.
    pub fn json(path: &Path, line: usize) -> impl FnOnce(serde_json::Error) -> ReviewError {
        let path = path.display().to_string();

        move |source| ReviewError::Json { path, line, source }
    }

    /// An answer that could not be turned into JSON.
    #[must_use]
    pub fn unserializable(error: serde_json::Error) -> ReviewError {
        ReviewError::BadRequest(format!("could not write JSON: {error}"))
    }

    /// The status this error is answered with.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            ReviewError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ReviewError::NotFound(_) => StatusCode::NOT_FOUND,
            ReviewError::Io { .. } | ReviewError::Json { .. } | ReviewError::Poisoned => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn every_error_says_what_and_where() -> Outcome {
        let path = Path::new("labels/flags.jsonl");
        let parse =
            serde_json::from_str::<serde_json::Value>("{").map_err(ReviewError::json(path, 3));
        let errors = [
            ReviewError::io(path)(io::Error::other("disk full")),
            parse.err().ok_or(ReviewError::Poisoned)?,
            ReviewError::BadRequest("phrase too short".to_string()),
            ReviewError::NotFound("record 9".to_string()),
            ReviewError::Poisoned,
        ];
        let shown: Vec<String> = errors.iter().map(ToString::to_string).collect();

        assert_eq!(shown[0], "labels/flags.jsonl: disk full");
        assert!(shown[1].starts_with("labels/flags.jsonl:3: "));
        assert_eq!(shown[2], "bad request: phrase too short");
        assert_eq!(shown[3], "not found: record 9");
        assert!(shown[4].contains("restart"));

        let unwritten = serde_json::from_str::<u8>("x").map_err(ReviewError::unserializable);

        assert!(unwritten.is_err_and(|error| {
            error
                .to_string()
                .starts_with("bad request: could not write JSON: ")
        }));

        let sources: Vec<bool> = errors
            .iter()
            .map(|error| error.source().is_some())
            .collect();

        assert_eq!(sources, [true, true, false, false, false]);

        Ok(())
    }

    #[test]
    fn statuses_follow_the_kind_of_error() {
        let io = ReviewError::io(Path::new("f"))(io::Error::other("gone"));

        assert_eq!(
            ReviewError::BadRequest(String::new()).status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ReviewError::NotFound(String::new()).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            ReviewError::Poisoned.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(io.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
