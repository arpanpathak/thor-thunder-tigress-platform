//! The error type for the review tool.

use std::{fmt, io, path::Path};

use crate::http::Status;

/// The result of anything in this crate that can fail.
pub type Outcome<T = ()> = Result<T, ReviewError>;

/// Anything that stops the tool, or that makes one request fail.
#[derive(Debug)]
pub enum ReviewError {
    /// A file could not be read or written.
    Io {
        /// The file involved.
        path: String,
        /// What the operating system reported.
        source: io::Error,
    },
    /// A line of JSONL did not parse.
    Json {
        /// The file involved.
        path: String,
        /// The one-based line number.
        line: usize,
        /// What the parser reported.
        source: serde_json::Error,
    },
    /// Reading the request or writing the answer failed: the client is gone.
    Connection(io::Error),
    /// The request asked for something malformed.
    BadRequest(String),
    /// The request asked for something that does not exist.
    NotFound(String),
    /// A thread panicked while it held the flags, so they can't be trusted.
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

    /// The status this error is answered with; `None` when there is no one
    /// left to answer.
    #[must_use]
    pub fn status(&self) -> Option<Status> {
        match self {
            ReviewError::BadRequest(_) => Some(Status::BadRequest),
            ReviewError::NotFound(_) => Some(Status::NotFound),
            ReviewError::Io { .. } | ReviewError::Json { .. } | ReviewError::Poisoned => {
                Some(Status::InternalError)
            }
            ReviewError::Connection(_) => None,
        }
    }
}

impl fmt::Display for ReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewError::Io { path, source } => write!(f, "{path}: {source}"),
            ReviewError::Json { path, line, source } => write!(f, "{path}:{line}: {source}"),
            ReviewError::Connection(source) => write!(f, "connection: {source}"),
            ReviewError::BadRequest(message) => write!(f, "bad request: {message}"),
            ReviewError::NotFound(message) => write!(f, "not found: {message}"),
            ReviewError::Poisoned => write!(
                f,
                "the flags are unavailable after a crash; restart the tool"
            ),
        }
    }
}

impl std::error::Error for ReviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReviewError::Io { source, .. } | ReviewError::Connection(source) => Some(source),
            ReviewError::Json { source, .. } => Some(source),
            ReviewError::BadRequest(_) | ReviewError::NotFound(_) | ReviewError::Poisoned => None,
        }
    }
}

impl From<io::Error> for ReviewError {
    fn from(source: io::Error) -> Self {
        ReviewError::Connection(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_error_keeps_its_source() {
        use std::error::Error as _;
        let source = serde_json::from_str::<u8>("x").err();
        let error = source.map(|source| ReviewError::Json {
            path: "f.jsonl".into(),
            line: 1,
            source,
        });
        assert!(error.as_ref().and_then(|error| error.source()).is_some());
    }
    use std::error::Error;

    #[test]
    fn every_error_says_what_and_where() -> Outcome {
        let path = Path::new("labels/flags.jsonl");
        let parse =
            serde_json::from_str::<serde_json::Value>("{").map_err(ReviewError::json(path, 3));
        let errors = [
            ReviewError::io(path)(io::Error::other("disk full")),
            parse.err().ok_or(ReviewError::Poisoned)?,
            ReviewError::from(io::Error::other("reset")),
            ReviewError::BadRequest("phrase too short".to_string()),
            ReviewError::NotFound("record 9".to_string()),
            ReviewError::Poisoned,
        ];
        let shown: Vec<String> = errors.iter().map(ToString::to_string).collect();
        assert_eq!(shown[0], "labels/flags.jsonl: disk full");
        assert!(shown[1].starts_with("labels/flags.jsonl:3: "));
        assert_eq!(shown[2], "connection: reset");
        assert_eq!(shown[3], "bad request: phrase too short");
        assert_eq!(shown[4], "not found: record 9");
        assert!(shown[5].contains("restart"));
        Ok(())
    }

    #[test]
    fn statuses_follow_the_kind_of_error() {
        assert_eq!(
            ReviewError::BadRequest(String::new()).status(),
            Some(Status::BadRequest)
        );
        assert_eq!(
            ReviewError::NotFound(String::new()).status(),
            Some(Status::NotFound)
        );
        assert_eq!(ReviewError::Poisoned.status(), Some(Status::InternalError));
        assert_eq!(ReviewError::from(io::Error::other("gone")).status(), None);
    }

    #[test]
    fn wrapped_errors_keep_their_source() {
        assert!(
            ReviewError::from(io::Error::other("gone"))
                .source()
                .is_some()
        );
        assert!(ReviewError::Poisoned.source().is_none());
    }
}
