//! The command line.
//!
//! ```text
//!   reinforcer [serve] [--port PORT] [--dataset NAME=RECORDS[,FLAGS[,REMOVED]]]...
//!   reinforcer RECORDS_JSONL [PORT] [FLAGS_JSONL] [REMOVED_JSONL]
//!   reinforcer scan  [TRAIN_JSONL]  [AUTO_FLAGS_JSONL]
//!   reinforcer apply [AUTO_FLAGS_JSONL] [FLAGS_JSONL]
//! ```
//!
//! With no `--dataset`, `serve` shows the training set, the teacher set and
//! the generated conversations, each with its own flags file, on one port.
//! The second form shows one file, as earlier versions did.

use std::path::{Path, PathBuf};

use crate::workspace::DatasetSpec;

/// The training file the scan reads when no argument is given.
const DEFAULT_TRAINING: &str = "data/train.jsonl";

/// The port the page is served on when no argument is given.
pub const DEFAULT_PORT: &str = "8787";

/// The flags file of the training set.
const DEFAULT_FLAGS: &str = "labels/slop_flags.jsonl";

/// The examples a build removed from the training set.
const DEFAULT_SLOP: &str = "data/slop.jsonl";

/// Where the scan writes the flags it suggests.
const DEFAULT_AUTO_FLAGS: &str = "labels/auto_flags.jsonl";

/// The datasets `serve` shows when none is named: name, records, flags, removed.
const DEFAULT_DATASETS: [(&str, &str, &str, &str); 3] = [
    ("train", DEFAULT_TRAINING, DEFAULT_FLAGS, DEFAULT_SLOP),
    (
        "teacher",
        "data/teacher.jsonl",
        "labels/teacher_flags.jsonl",
        "data/teacher_removed.jsonl",
    ),
    (
        "conversations",
        "data/conversations.jsonl",
        "labels/conversation_flags.jsonl",
        "data/conversations_removed.jsonl",
    ),
];

/// The usage text.
pub const USAGE: &str = "usage: reinforcer [serve] [--port PORT] [--dataset NAME=RECORDS[,FLAGS[,REMOVED]]]...\n       reinforcer RECORDS_JSONL [PORT] [FLAGS_JSONL] [REMOVED_JSONL]\n       reinforcer scan [TRAIN_JSONL] [AUTO_FLAGS_JSONL]\n       reinforcer apply [AUTO_FLAGS_JSONL] [FLAGS_JSONL]";

/// What the tool was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Serve the review page.
    Serve {
        /// The port on 127.0.0.1.
        port: String,
        /// The datasets the page can switch between, first shown first.
        datasets: Vec<DatasetSpec>,
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
    /// The arguments could not be read; the reason.
    Usage(String),
}

impl Command {
    /// Reads `arguments` (without the program name).
    #[must_use]
    pub fn from_args(arguments: impl IntoIterator<Item = String>) -> Command {
        let arguments: Vec<String> = arguments.into_iter().collect();
        let Some((first, rest)) = arguments.split_first() else {
            return serve(&[]);
        };
        match first.as_str() {
            "serve" => serve(rest),
            "scan" => Command::Scan {
                training: nth_or(rest, 0, DEFAULT_TRAINING).into(),
                out: nth_or(rest, 1, DEFAULT_AUTO_FLAGS).into(),
            },
            "apply" => Command::Apply {
                suggestions: nth_or(rest, 0, DEFAULT_AUTO_FLAGS).into(),
                flags: nth_or(rest, 1, DEFAULT_FLAGS).into(),
            },
            option if option.starts_with("--") => serve(&arguments),
            records => Command::Serve {
                port: nth_or(rest, 0, DEFAULT_PORT),
                datasets: vec![DatasetSpec::new(
                    "train",
                    Path::new(records),
                    Path::new(&nth_or(rest, 1, DEFAULT_FLAGS)),
                    Path::new(&nth_or(rest, 2, DEFAULT_SLOP)),
                )],
            },
        }
    }
}

fn nth_or(arguments: &[String], position: usize, default: &str) -> String {
    arguments
        .get(position)
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

fn serve(options: &[String]) -> Command {
    let mut port = DEFAULT_PORT.to_string();
    let mut datasets = Vec::new();
    let mut rest = options.iter();
    while let Some(option) = rest.next() {
        let Some(value) = rest.next() else {
            return Command::Usage(format!("{option} needs a value"));
        };
        match option.as_str() {
            "--port" => port.clone_from(value),
            "--dataset" => match dataset(value) {
                Some(spec) => datasets.push(spec),
                None => {
                    return Command::Usage(format!(
                        "--dataset {value}: expected NAME=RECORDS[,FLAGS[,REMOVED]]"
                    ));
                }
            },
            other => return Command::Usage(format!("unknown option {other}")),
        }
    }
    if datasets.is_empty() {
        datasets = DEFAULT_DATASETS
            .iter()
            .map(|&(name, records, flags, removed)| {
                DatasetSpec::new(
                    name,
                    Path::new(records),
                    Path::new(flags),
                    Path::new(removed),
                )
            })
            .collect();
    }
    Command::Serve { port, datasets }
}

/// Reads `NAME=RECORDS[,FLAGS[,REMOVED]]`. Missing files default to
/// `labels/NAME_flags.jsonl` and `data/NAME_removed.jsonl`.
fn dataset(value: &str) -> Option<DatasetSpec> {
    let (name, files) = value.split_once('=')?;
    let mut files = files.split(',').filter(|file| !file.is_empty());
    let records = files.next()?;
    let flags = files
        .next()
        .map_or_else(|| format!("labels/{name}_flags.jsonl"), str::to_string);
    let removed = files
        .next()
        .map_or_else(|| format!("data/{name}_removed.jsonl"), str::to_string);
    (!name.is_empty()).then(|| {
        DatasetSpec::new(
            name,
            Path::new(records),
            Path::new(&flags),
            Path::new(&removed),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(words: &[&str]) -> Command {
        Command::from_args(words.iter().map(ToString::to_string))
    }

    fn names(command: &Command) -> Vec<String> {
        match command {
            Command::Serve { datasets, .. } => {
                datasets.iter().map(|spec| spec.name.clone()).collect()
            }
            Command::Scan { .. } | Command::Apply { .. } | Command::Usage(_) => Vec::new(),
        }
    }

    fn specs(command: Command) -> Vec<DatasetSpec> {
        match command {
            Command::Serve { datasets, .. } => datasets,
            Command::Scan { .. } | Command::Apply { .. } | Command::Usage(_) => Vec::new(),
        }
    }

    #[test]
    fn serves_every_default_dataset_when_nothing_is_given() {
        assert_eq!(names(&command(&[])), ["train", "teacher", "conversations"]);
        assert!(names(&command(&["scan"])).is_empty() && specs(command(&["apply"])).is_empty());
        assert_eq!(
            names(&command(&["serve", "--port", "9000"])),
            ["train", "teacher", "conversations"]
        );
        assert!(
            matches!(command(&["--port", "9000"]), Command::Serve { port, .. } if port == "9000")
        );
    }

    #[test]
    fn reads_named_datasets() {
        let served = command(&[
            "serve",
            "--dataset",
            "mine=a.jsonl,f.jsonl,r.jsonl",
            "--dataset",
            "other=b.jsonl",
        ]);
        assert!(matches!(&served, Command::Serve { port, .. } if port == DEFAULT_PORT));
        let datasets = specs(served);
        assert_eq!(
            datasets[0],
            DatasetSpec::new(
                "mine",
                Path::new("a.jsonl"),
                Path::new("f.jsonl"),
                Path::new("r.jsonl")
            )
        );
        assert_eq!(datasets[1].flags, PathBuf::from("labels/other_flags.jsonl"));
        assert_eq!(
            datasets[1].removed,
            PathBuf::from("data/other_removed.jsonl")
        );
    }

    #[test]
    fn serves_one_file_the_old_way() {
        let served = command(&["data/conversations.jsonl", "8788", "labels/c.jsonl"]);
        assert!(matches!(&served, Command::Serve { port, .. } if port == "8788"));
        let datasets = specs(served);
        assert_eq!(
            datasets,
            [DatasetSpec::new(
                "train",
                Path::new("data/conversations.jsonl"),
                Path::new("labels/c.jsonl"),
                Path::new(DEFAULT_SLOP)
            )]
        );
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        let usage = |words: &[&str]| matches!(command(words), Command::Usage(_));
        assert!(usage(&["serve", "--port"]));
        assert!(usage(&["serve", "--colour", "dark"]));
        assert!(usage(&["serve", "--dataset", "no-equals"]));
        assert!(usage(&["serve", "--dataset", "=a.jsonl"]));
        assert!(usage(&["serve", "--dataset", "x="]));
    }

    #[test]
    fn reads_scan_and_apply() {
        assert_eq!(
            command(&["scan"]),
            Command::Scan {
                training: DEFAULT_TRAINING.into(),
                out: DEFAULT_AUTO_FLAGS.into()
            }
        );
        assert_eq!(
            command(&["apply", "a.jsonl", "f.jsonl"]),
            Command::Apply {
                suggestions: "a.jsonl".into(),
                flags: "f.jsonl".into()
            }
        );
    }
}
