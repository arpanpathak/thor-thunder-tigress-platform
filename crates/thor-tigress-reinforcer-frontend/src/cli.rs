//! The command line.
//!
//! ```text
//!   reinforcer [TRAIN_JSONL] [PORT] [FLAGS_JSONL] [SLOP_JSONL]
//!   reinforcer scan  [TRAIN_JSONL]  [AUTO_FLAGS_JSONL]
//!   reinforcer apply [AUTO_FLAGS_JSONL] [FLAGS_JSONL]
//! ```

use std::path::PathBuf;

/// The training file the tool reads when no argument is given.
const DEFAULT_TRAINING: &str = "data/train.jsonl";

/// The port the page is served on when no argument is given.
const DEFAULT_PORT: &str = "8080";

/// The flags file the tool writes when no argument is given.
const DEFAULT_FLAGS: &str = "labels/slop_flags.jsonl";

/// The examples already removed as slop, shown alongside the training set.
const DEFAULT_SLOP: &str = "data/slop.jsonl";

/// Where the scan writes the flags it suggests.
const DEFAULT_AUTO_FLAGS: &str = "labels/auto_flags.jsonl";

/// What the tool was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Serve the review page.
    Serve {
        /// The training file.
        training: PathBuf,
        /// The port on 127.0.0.1.
        port: String,
        /// The reviewer's flags.
        flags: PathBuf,
        /// The examples a build removed.
        slop: PathBuf,
    },
    /// Suggest flags for the whole training file.
    Scan {
        /// The training file.
        training: PathBuf,
        /// Where the suggestions go.
        out: PathBuf,
    },
    /// Turn suggestions into flags a person then reviews.
    Apply {
        /// The suggestions a scan wrote.
        suggestions: PathBuf,
        /// The reviewer's flags.
        flags: PathBuf,
    },
}

impl Command {
    /// Reads `arguments` (without the program name). Every argument is
    /// optional, in order; a missing one takes its default.
    #[must_use]
    pub fn from_args(arguments: impl IntoIterator<Item = String>) -> Command {
        let mut arguments = arguments.into_iter();
        let first = arguments.next();
        let mut next_or = |default: &str| arguments.next().unwrap_or_else(|| default.to_string());
        match first.as_deref() {
            Some("scan") => Command::Scan {
                training: next_or(DEFAULT_TRAINING).into(),
                out: next_or(DEFAULT_AUTO_FLAGS).into(),
            },
            Some("apply") => Command::Apply {
                suggestions: next_or(DEFAULT_AUTO_FLAGS).into(),
                flags: next_or(DEFAULT_FLAGS).into(),
            },
            _ => Command::Serve {
                training: first.unwrap_or_else(|| DEFAULT_TRAINING.to_string()).into(),
                port: next_or(DEFAULT_PORT),
                flags: next_or(DEFAULT_FLAGS).into(),
                slop: next_or(DEFAULT_SLOP).into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(words: &[&str]) -> Command {
        Command::from_args(words.iter().map(ToString::to_string))
    }

    #[test]
    fn serves_with_defaults_when_nothing_is_given() {
        assert_eq!(
            command(&[]),
            Command::Serve {
                training: DEFAULT_TRAINING.into(),
                port: DEFAULT_PORT.to_string(),
                flags: DEFAULT_FLAGS.into(),
                slop: DEFAULT_SLOP.into(),
            }
        );
    }

    #[test]
    fn serves_what_is_given_in_order() {
        assert_eq!(
            command(&["data/conversations.jsonl", "8788", "labels/c.jsonl"]),
            Command::Serve {
                training: "data/conversations.jsonl".into(),
                port: "8788".to_string(),
                flags: "labels/c.jsonl".into(),
                slop: DEFAULT_SLOP.into(),
            }
        );
    }

    #[test]
    fn reads_scan_and_apply() {
        assert_eq!(command(&["scan"]), Command::Scan { training: DEFAULT_TRAINING.into(), out: DEFAULT_AUTO_FLAGS.into() });
        assert_eq!(
            command(&["apply", "a.jsonl", "f.jsonl"]),
            Command::Apply { suggestions: "a.jsonl".into(), flags: "f.jsonl".into() }
        );
    }
}
