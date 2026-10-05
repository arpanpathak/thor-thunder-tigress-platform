//! The error type for the review tool.

use std::{fmt, io, path::Path};

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
    /// The request asked for something malformed.
    BadRequest(String),
    /// The request asked for something that does not exist.
    NotFound(String),
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

    /// The status line this error is answered with.
    pub fn status(&self) -> &'static str {
        match self {
            ReviewError::BadRequest(_) => "400 Bad Request",
            ReviewError::NotFound(_) => "404 Not Found",
            ReviewError::Io { .. } | ReviewError::Json { .. } => "500 Internal Server Error",
        }
    }
}

impl fmt::Display for ReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewError::Io { path, source } => write!(f, "{path}: {source}"),
            ReviewError::Json {
                path,
                line,
                source,
            } => write!(f, "{path}:{line}: {source}"),
            ReviewError::BadRequest(message) => write!(f, "bad request: {message}"),
            ReviewError::NotFound(message) => write!(f, "not found: {message}"),
        }
    }
}

impl std::error::Error for ReviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReviewError::Io { source, .. } => Some(source),
            ReviewError::Json { source, .. } => Some(source),
            ReviewError::BadRequest(_) | ReviewError::NotFound(_) => None,
        }
    }
}
