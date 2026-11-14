//! The one error type of the crate.

use std::{io, path::PathBuf};

use thiserror::Error;

/// Everything that can go wrong in `lasso`.
#[derive(Debug, Error)]
pub enum DistillError {
    /// A file could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file involved.
        path: PathBuf,
        /// What the operating system reported.
        source: io::Error,
    },
    /// A line of the training file is not valid JSON.
    #[error("{}:{line}: {source}", path.display())]
    Json {
        /// The file involved.
        path: PathBuf,
        /// The 1-based line number.
        line: usize,
        /// What the parser reported.
        source: serde_json::Error,
    },
    /// The model server could not be reached or answered with an error.
    #[error("model server: {0}")]
    Server(String),
    /// The command line was not understood.
    #[error("{0}")]
    Usage(String),
    /// A program this one runs, such as `gh` or `git`, failed.
    #[error("{0}")]
    Command(String),
}

impl DistillError {
    /// A closure that wraps an I/O error with the path it happened on.
    pub fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> DistillError {
        let path = path.into();

        move |source| DistillError::Io { path, source }
    }

    /// A closure that wraps a JSON error with the file and 1-based line it was on.
    pub fn json(
        path: impl Into<PathBuf>,
        line: usize,
    ) -> impl FnOnce(serde_json::Error) -> DistillError {
        let path = path.into();

        move |source| DistillError::Json { path, line, source }
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
        let command = DistillError::Command("gh: not logged in".to_string());
        let parsed = serde_json::from_str::<u8>("x")
            .map_err(DistillError::json("runs.jsonl", 2))
            .err();

        assert_eq!(io.to_string(), "data/train.jsonl: disk full");
        assert!(
            json.as_ref()
                .is_some_and(|error| error.to_string().starts_with("t.jsonl:3: "))
        );
        assert_eq!(server.to_string(), "model server: engine not loaded");
        assert_eq!(usage.to_string(), "usage: lasso");
        assert_eq!(command.to_string(), "gh: not logged in");
        assert!(parsed.is_some_and(|error| error.to_string().starts_with("runs.jsonl:2: ")));
        assert!(io.source().is_some());
        assert!(json.as_ref().and_then(|error| error.source()).is_some());
        assert!(server.source().is_none() && usage.source().is_none());
    }
}
