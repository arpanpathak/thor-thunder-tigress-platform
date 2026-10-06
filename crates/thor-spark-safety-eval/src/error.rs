//! The one error type of the crate.

use std::{fmt, io, path::PathBuf};

/// The result of anything in this crate that can fail.
pub type Outcome<T = ()> = Result<T, EvalError>;

/// Everything that can go wrong while scoring.
#[derive(Debug)]
pub enum EvalError {
    /// A file could not be read or written.
    Io {
        /// The file involved.
        path: PathBuf,
        /// What the operating system reported.
        source: io::Error,
    },
    /// A line of a JSONL file is not valid JSON.
    Json {
        /// The file involved.
        path: PathBuf,
        /// The 1-based line number.
        line: usize,
        /// What the parser reported.
        source: serde_json::Error,
    },
    /// A JSONL record has no string in the field the answer is read from.
    MissingField {
        /// The file involved.
        path: PathBuf,
        /// The 1-based line number.
        line: usize,
        /// The field that was asked for.
        field: String,
    },
    /// The command line was not understood.
    Usage(String),
}

impl EvalError {
    /// A closure that wraps an I/O error with the path it happened on.
    pub fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> EvalError {
        let path = path.into();
        move |source| EvalError::Io { path, source }
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            EvalError::Json { path, line, source } => {
                write!(formatter, "{}:{line}: {source}", path.display())
            }
            EvalError::MissingField { path, line, field } => {
                write!(formatter, "{}:{line}: no string field \"{field}\"", path.display())
            }
            EvalError::Usage(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for EvalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EvalError::Io { source, .. } => Some(source),
            EvalError::Json { source, .. } => Some(source),
            EvalError::MissingField { .. } | EvalError::Usage(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn every_error_names_the_file_and_line() {
        let json = serde_json::from_str::<serde_json::Value>("{").map_err(|source| EvalError::Json {
            path: PathBuf::from("run.jsonl"),
            line: 4,
            source,
        });
        let errors = [
            EvalError::io("answers.md")(io::Error::other("denied")),
            EvalError::MissingField { path: PathBuf::from("run.jsonl"), line: 2, field: "text".to_string() },
            EvalError::Usage("usage: spark".to_string()),
        ];
        let shown: Vec<String> = errors.iter().map(ToString::to_string).collect();
        assert_eq!(shown, ["answers.md: denied", "run.jsonl:2: no string field \"text\"", "usage: spark"]);
        assert!(json.is_err_and(|error| error.to_string().starts_with("run.jsonl:4: ") && error.source().is_some()));
        assert!(errors[1].source().is_none() && errors[0].source().is_some());
    }
}
