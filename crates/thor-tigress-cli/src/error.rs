//! The errors the `thor` command reports, each naming what it was working on.

use std::path::PathBuf;

/// Everything that can stop a `thor` command.
#[derive(Debug, thiserror::Error)]
pub enum ThorError {
    /// A file or folder could not be read or written.
    #[error("{path}: {source}")]
    File {
        /// The file or folder.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },

    /// A web request failed, or the server answered with an error status.
    #[error("{url}: {source}")]
    Http {
        /// The address asked for.
        url: String,
        /// What the HTTP client reported.
        source: Box<ureq::Error>,
    },

    /// A reply or a file was not the JSON it should be.
    #[error("{what} is not the JSON expected: {source}")]
    Json {
        /// What was being read.
        what: String,
        /// Where parsing stopped.
        source: serde_json::Error,
    },

    /// A program such as `systemctl` could not be run.
    #[error("could not run {program}: {source}")]
    Program {
        /// The program.
        program: String,
        /// What the operating system said.
        source: std::io::Error,
    },

    /// A program ran and reported a failure.
    #[error("{program} failed: {output}")]
    ProgramFailed {
        /// The command line, without secrets.
        program: String,
        /// What it printed on standard error.
        output: String,
    },

    /// A menu or question could not be shown or was cancelled.
    #[error("{0}")]
    Prompt(#[from] inquire::InquireError),

    /// The command line asked for something this command does not offer.
    #[error("{0}\n\n{usage}", usage = crate::cli::USAGE)]
    Usage(String),

    /// A condition for the request is not met. The message names it and the next step.
    #[error("{0}")]
    Condition(String),
}

/// The result type of every fallible function in this crate.
pub type Outcome<T> = Result<T, ThorError>;

/// Wraps an input or output error with the path it concerns.
pub fn file_error(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> ThorError + '_ {
    move |source| ThorError::File {
        path: path.to_path_buf(),
        source,
    }
}
