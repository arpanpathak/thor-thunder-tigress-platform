//! The error type for building the training set.
//!
//! [`DataError`] stops the data build. Anything less serious, such as an
//! example that is too short, is a [`crate::example::SkipReason`] instead.

use std::{fmt, io, path::Path};

use crate::teacher::FormatError;

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
    /// A teacher entry does not follow its format.
    Format {
        /// The file and entry number.
        origin: String,
        /// What is wrong with it.
        error: FormatError,
    },
}

impl DataError {
    /// A converter for `map_err` that keeps the path next to the I/O error.
    pub fn io(path: &Path) -> impl FnOnce(io::Error) -> DataError {
        let path = path.display().to_string();
        move |source| DataError::Io { path, source }
    }

    /// A converter for `map_err` that keeps the entry's origin next to its format error.
    pub fn format(origin: &str) -> impl FnOnce(FormatError) -> DataError {
        let origin = origin.to_string();
        move |error| DataError::Format { origin, error }
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
            DataError::Format { origin, error } => write!(f, "{origin}: {error}"),
        }
    }
}

impl std::error::Error for DataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DataError::Io { source, .. } => Some(source),
            DataError::Json(error) => Some(error),
            DataError::Format { error, .. } => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teacher::FormatError;
    use std::error::Error as _;

    #[test]
    fn every_error_names_its_cause_and_keeps_its_source() {
        let io = DataError::io(Path::new("data/train.jsonl"))(io::Error::other("disk full"));
        let json = serde_json::from_str::<u8>("x").err().map(DataError::from);
        let format = DataError::format("a.md#2")(FormatError::NoTurns);
        assert_eq!(io.to_string(), "data/train.jsonl: disk full");
        assert!(json.as_ref().is_some_and(|error| error.to_string().starts_with("JSON: ")));
        assert_eq!(format.to_string(), "a.md#2: no ### User or ### Assistant section");
        assert!(io.source().is_some());
        assert!(json.as_ref().and_then(|error| error.source()).is_some());
        assert!(format.source().is_some());
    }
}
