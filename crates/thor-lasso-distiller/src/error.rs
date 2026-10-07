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

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn every_error_says_what_and_where() {
        let io = DistillError::io("data/train.jsonl")(io::Error::other("disk full"));
        let json = serde_json::from_str::<u8>("x")
            .err()
            .map(|source| DistillError::Json {
                path: "t.jsonl".into(),
                line: 3,
                source,
            });
        let server = DistillError::Server("engine not loaded".to_string());
        let usage = DistillError::Usage("usage: lasso".to_string());
        assert_eq!(io.to_string(), "data/train.jsonl: disk full");
        assert!(
            json.as_ref()
                .is_some_and(|error| error.to_string().starts_with("t.jsonl:3: "))
        );
        assert_eq!(server.to_string(), "model server: engine not loaded");
        assert_eq!(usage.to_string(), "usage: lasso");
        assert!(io.source().is_some());
        assert!(json.as_ref().and_then(|error| error.source()).is_some());
        assert!(server.source().is_none() && usage.source().is_none());
    }
}
