//! The one error type of the crate.

use std::{fmt, io, path::PathBuf};

/// Everything that can go wrong while building conversations.
#[derive(Debug)]
pub enum DistillError {
    /// A file could not be read or written.
    Io {
        /// The file involved.
        path: PathBuf,
        /// What the operating system reported.
        source: io::Error,
    },
    /// A line of the training file is not valid JSON.
    Json {
        /// The file involved.
        path: PathBuf,
        /// The 1-based line number.
        line: usize,
        /// What the parser reported.
        source: serde_json::Error,
    },
    /// The model server could not be reached or answered with an error.
    Server(String),
    /// The command line was not understood.
    Usage(String),
}

impl DistillError {
    /// A closure that wraps an I/O error with the path it happened on.
    pub fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> DistillError {
        let path = path.into();
        move |source| DistillError::Io { path, source }
    }
}

impl fmt::Display for DistillError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DistillError::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            DistillError::Json { path, line, source } => {
                write!(formatter, "{}:{line}: {source}", path.display())
            }
            DistillError::Server(message) => write!(formatter, "model server: {message}"),
            DistillError::Usage(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for DistillError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DistillError::Io { source, .. } => Some(source),
            DistillError::Json { source, .. } => Some(source),
            DistillError::Server(_) | DistillError::Usage(_) => None,
        }
    }
}
