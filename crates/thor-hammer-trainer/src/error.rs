//! The error type for building the training set.
//!
//! [`DataError`] stops the data build. Anything less serious, such as an
//! example that is too short, is a [`crate::example::SkipReason`] instead.

use std::{fmt, io, path::Path};

/// A failure that stops the build.
#[derive(Debug)]
pub enum DataError {
    /// A file or folder could not be read or written.
    Io {
        /// The file or folder involved.
        path: String,
        /// What the operating system reported.
        source: io::Error,
    },
    /// The chat export did not parse, or an example could not be serialized.
    Json(serde_json::Error),
}

impl DataError {
    /// A converter for `map_err` that keeps the path next to the I/O error.
    pub fn io(path: &Path) -> impl FnOnce(io::Error) -> DataError {
        let path = path.display().to_string();
        move |source| DataError::Io { path, source }
    }
}

impl From<serde_json::Error> for DataError {
    fn from(error: serde_json::Error) -> Self {
        DataError::Json(error)
    }
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::Io { path, source } => write!(f, "{path}: {source}"),
            DataError::Json(error) => write!(f, "JSON: {error}"),
        }
    }
}

impl std::error::Error for DataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DataError::Io { source, .. } => Some(source),
            DataError::Json(error) => Some(error),
        }
    }
}
