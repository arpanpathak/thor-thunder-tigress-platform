//! The `lasso` command line, as a library so it can be tested.
//!
//! ```text
//! lasso conversations [--train data/train.jsonl] [--out data/conversations.jsonl]
//!                     [--books trpl,rbe,...] [--turns 3] [--limit N]
//!                     [--server 127.0.0.1:8000] [--model NAME] [--dry-run]
//! ```
//!
//! Runs are incremental: conversations already in `--out` are skipped, and
//! `--limit N` generates N new ones, so the work can be done in small batches
//! at times that suit the people sharing the model server.
//!
//! An access key for the server is read from the `LASSO_KEY` environment
//! variable, so it never appears in the command line or the shell history.
//!
//! `--dry-run` calls no model: it prints how many conversations and turns the
//! books give, and writes the first prompts to `OUT.prompts.jsonl` so they can
//! be read before any generation is paid for. Without it, every conversation
//! is appended to `--out` as soon as it is done, so a long run that stops
//! keeps its work.

use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use serde_json::json;

use crate::{
    client::Client,
    conversations::{self, Passage},
    error::DistillError,
};

/// The address of the model server unless `--server` says otherwise.
pub const DEFAULT_SERVER: &str = "127.0.0.1:8000";

/// Conversations whose first prompt a dry run writes unless `--limit` says otherwise.
const DRY_RUN_PROMPTS: usize = 20;

/// Generated conversations between two progress lines.
const PROGRESS_EVERY: usize = 10;

/// The command line of `lasso conversations`.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// The training file with the book passages.
    pub train: PathBuf,
    /// Where conversations are appended.
    pub out: PathBuf,
    /// The book folders to use; every one when empty.
    pub books: Vec<String>,
    /// Turns per conversation.
    pub turns: usize,
    /// How many new conversations to make at most.
    pub limit: Option<usize>,
    /// The model server.
    pub client: Client,
    /// True to write prompts instead of calling the model.
    pub dry_run: bool,
}

/// Reads the arguments after the program name and runs the command,
/// writing progress to `out`.
///
/// # Errors
///
/// [`DistillError::Usage`] for arguments that can't be read, and any error of
/// the run itself.
pub fn main_with(
    arguments: &[String],
    key: Option<String>,
    out: &mut impl Write,
) -> Result<(), DistillError> {
    match arguments.split_first() {
        Some((command, rest)) if command == "conversations" => run(&options(rest, key)?, out),
        _ => Err(DistillError::Usage(usage())),
    }
}

/// The usage text.
#[must_use]
pub fn usage() -> String {
    "usage: lasso conversations [--train FILE] [--out FILE] [--books a,b] [--turns N] \
     [--limit N] [--server HOST:PORT] [--model NAME] [--dry-run]"
        .to_string()
}

/// Reads the options of `lasso conversations`, with the server's access key.
///
/// # Errors
///
/// [`DistillError::Usage`] for an unknown option, a missing value or a value
/// that should be a number and is not.
pub fn options(arguments: &[String], key: Option<String>) -> Result<Options, DistillError> {
    let mut options = Options {
        train: PathBuf::from("data/train.jsonl"),
        out: PathBuf::from("data/conversations.jsonl"),
        books: Vec::new(),
        turns: 3,
        limit: None,
        client: Client {
            address: DEFAULT_SERVER.to_string(),
            model: "teacher".to_string(),
            key: key.filter(|key| !key.is_empty()),
        },
        dry_run: false,
    };
    let mut rest = arguments.iter();

    while let Some(argument) = rest.next() {
        let mut value = || {
            rest.next()
                .cloned()
                .ok_or_else(|| DistillError::Usage(format!("{argument} needs a value")))
        };
        let number = |text: String| {
            text.parse::<usize>()
                .map_err(|_| DistillError::Usage(format!("{argument} needs a number, not {text}")))
        };

        match argument.as_str() {
            "--train" => options.train = PathBuf::from(value()?),
            "--out" => options.out = PathBuf::from(value()?),
            "--books" => options.books = value()?.split(',').map(str::to_string).collect(),
            "--turns" => options.turns = number(value()?)?,
            "--limit" => options.limit = Some(number(value()?)?),
            "--server" => options.client.address = value()?,
            "--model" => options.client.model = value()?,
            "--dry-run" => options.dry_run = true,
            other => {
                return Err(DistillError::Usage(format!(
                    "unknown option {other}\n{}",
                    usage()
                )));
            }
        }
    }

    Ok(options)
}

/// Plans the conversations, skips the ones already written, and writes the
/// prompts (dry run) or generates the rest.
///
/// # Errors
///
/// [`DistillError::Io`] or [`DistillError::Json`] for the files, and
/// [`DistillError::Server`] when the model server fails.
pub fn run(options: &Options, out: &mut impl Write) -> Result<(), DistillError> {
    let passages = conversations::read_passages(&options.train, &options.books)?;
    let planned = conversations::plan(passages, options.turns);
    let counts = conversations::plan_counts(&planned);
    let turns: usize = planned.iter().map(Vec::len).sum();
    let console = DistillError::io("standard output");
    writeln!(
        out,
        "{} conversations, {turns} turns, from {} books and doc sets",
        planned.len(),
        counts.len()
    )
    .map_err(console)?;

    for (book, (talks, turns)) in &counts {
        writeln!(
            out,
            "  {book:<44} {talks:>5} conversations {turns:>6} turns"
        )
        .map_err(DistillError::io("standard output"))?;
    }
    let finished = finished_keys(&options.out)?;
    let remaining: Vec<&Vec<Passage>> = planned
        .iter()
        .filter(|sections| !finished.contains(&conversations::conversation_key(sections)))
        .collect();
    let done = planned.len() - remaining.len();
    writeln!(
        out,
        "{done} already in {}, {} left",
        options.out.display(),
        remaining.len()
    )
    .map_err(DistillError::io("standard output"))?;
    let chosen: Vec<&Vec<Passage>> = remaining
        .into_iter()
        .take(options.limit.unwrap_or(usize::MAX))
        .collect();

    if options.dry_run {
        write_prompts(options, &chosen, out)
    } else {
        generate(options, &chosen, out)
    }
}

fn write_prompts(
    options: &Options,
    chosen: &[&Vec<Passage>],
    out: &mut impl Write,
) -> Result<(), DistillError> {
    let path = options.out.with_extension("prompts.jsonl");
    let lines: Vec<String> = chosen
        .iter()
        .take(options.limit.unwrap_or(DRY_RUN_PROMPTS))
        .filter_map(|conversation| conversation.first())
        .map(|passage| {
            let messages: Vec<serde_json::Value> = conversations::question_prompt(&[], passage)
                .iter()
                .map(|message| json!({ "role": message.role, "content": message.content }))
                .collect();
            json!({ "origin": passage.origin, "messages": messages }).to_string()
        })
        .collect();
    fs::write(&path, lines.join("\n") + "\n").map_err(DistillError::io(&path))?;
    writeln!(
        out,
        "dry run: wrote {} first-turn prompts to {}",
        lines.len(),
        path.display()
    )
    .map_err(DistillError::io("standard output"))
}

/// The keys of the conversations an earlier run already wrote to `path`.
fn finished_keys(path: &Path) -> Result<HashSet<String>, DistillError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(error) => return Err(DistillError::io(path)(error)),
    };
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|record| {
            record
                .get("key")
                .and_then(|key| key.as_str())
                .map(str::to_string)
        })
        .collect())
}

fn generate(
    options: &Options,
    chosen: &[&Vec<Passage>],
    out: &mut impl Write,
) -> Result<(), DistillError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&options.out)
        .map_err(DistillError::io(&options.out))?;
    let started = Instant::now();
    let (mut kept, mut rejected) = (0usize, 0usize);

    for (done, sections) in chosen.iter().enumerate() {
        let conversation = conversations::converse(&options.client, sections)?;
        kept += conversation.turns.len();
        rejected += conversation.rejected.len();

        if !conversation.turns.is_empty() {
            let line = conversation.to_json(&options.client.model);
            writeln!(file, "{line}").map_err(DistillError::io(&options.out))?;
        }
        let count = done + 1;

        if count % PROGRESS_EVERY == 0 || count == chosen.len() {
            let seconds = started.elapsed().as_secs_f64();
            let line = format!(
                "{count}/{} conversations, {kept} questions kept, {rejected} rejected, {seconds:.0} s",
                chosen.len()
            );
            writeln!(out, "{line}").map_err(DistillError::io("standard output"))?;
        }
    }
    writeln!(out, "wrote {}", options.out.display()).map_err(DistillError::io("standard output"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeModel;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    struct Folder {
        path: PathBuf,
    }

    impl Folder {
        fn new(name: &str) -> Result<Self, DistillError> {
            let path =
                std::env::temp_dir().join(format!("thor-lasso-cli-{name}-{}", std::process::id()));
            fs::create_dir_all(&path).map_err(DistillError::io(&path))?;
            let train = path.join("train.jsonl");
            let lines = [
                r##"{"instruction":"","response":"# Vectors\n\nUse push to grow a vector.","origin":"trpl/src/ch08.md"}"##,
                r##"{"instruction":"","response":"# Strings\n\nUse push_str to append.","origin":"trpl/src/ch08.md"}"##,
                r##"{"instruction":"","response":"# Loops\n\nUse for to loop.","origin":"rbe/src/loops.md"}"##,
                r##"{"instruction":"Q","response":"A","origin":"chat"}"##,
            ];
            fs::write(&train, lines.join("\n")).map_err(DistillError::io(&train))?;
            Ok(Self { path })
        }

        fn arguments(&self, extra: &[&str]) -> Vec<String> {
            let train = self.path.join("train.jsonl").display().to_string();
            let out = self.path.join("conversations.jsonl").display().to_string();
            let mut arguments = words(&["conversations", "--train", &train, "--out", &out]);
            arguments.extend(words(extra));
            arguments
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn reads_every_option() -> Result<(), DistillError> {
        let arguments = words(&[
            "--train",
            "t",
            "--out",
            "o",
            "--books",
            "trpl,rbe",
            "--turns",
            "2",
            "--limit",
            "5",
            "--server",
            "h:1",
            "--model",
            "m",
            "--dry-run",
        ]);
        let options = options(&arguments, Some("key".to_string()))?;
        assert_eq!(options.books, ["trpl", "rbe"]);
        assert_eq!(
            (options.turns, options.limit, options.dry_run),
            (2, Some(5), true)
        );
        assert_eq!(
            options.client,
            Client {
                address: "h:1".to_string(),
                model: "m".to_string(),
                key: Some("key".to_string())
            }
        );
        assert_eq!(super::options(&[], Some(String::new()))?.client.key, None);
        Ok(())
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        let refused = |list: &[&str]| {
            options(&words(list), None)
                .err()
                .map(|error| error.to_string())
        };
        assert_eq!(
            refused(&["--turns"]).as_deref(),
            Some("--turns needs a value")
        );
        assert_eq!(
            refused(&["--limit", "many"]).as_deref(),
            Some("--limit needs a number, not many")
        );
        assert!(
            refused(&["--colour"])
                .is_some_and(|message| message.starts_with("unknown option --colour"))
        );
        let mut out = Vec::new();
        assert!(matches!(
            main_with(&words(&["help"]), None, &mut out),
            Err(DistillError::Usage(_))
        ));
    }

    #[test]
    fn a_dry_run_writes_prompts_and_counts() -> Result<(), DistillError> {
        let folder = Folder::new("dry")?;
        let mut out = Vec::new();
        main_with(&folder.arguments(&["--dry-run"]), None, &mut out)?;
        let printed = String::from_utf8_lossy(&out).into_owned();
        assert!(printed.starts_with("2 conversations, 3 turns, from 2 books and doc sets"));
        assert!(printed.contains("dry run: wrote 2 first-turn prompts"));
        let prompts = fs::read_to_string(folder.path.join("conversations.prompts.jsonl"))
            .map_err(DistillError::io(&folder.path))?;
        assert_eq!(prompts.lines().count(), 2);
        Ok(())
    }

    #[test]
    fn generates_and_then_skips_what_is_written() -> Result<(), DistillError> {
        let folder = Folder::new("generate")?;
        let model = FakeModel::start(&[
            "How do I grow a vector?",
            "yes",
            "How do I append text?",
            "yes",
            "x",
            "y",
            "z",
        ])?;
        let server = model.client.address.clone();
        let mut out = Vec::new();
        main_with(&folder.arguments(&["--server", &server]), None, &mut out)?;
        let written = fs::read_to_string(folder.path.join("conversations.jsonl"))
            .map_err(DistillError::io(&folder.path))?;
        assert_eq!(written.lines().count(), 1);
        let printed = String::from_utf8_lossy(&out).into_owned();
        assert!(
            printed.contains("2/2 conversations, 2 questions kept, 3 rejected"),
            "{printed}"
        );
        assert!(!printed.contains("1/2 conversations"));
        let mut again = Vec::new();
        main_with(
            &folder.arguments(&["--books", "trpl", "--dry-run"]),
            None,
            &mut again,
        )?;
        assert!(String::from_utf8_lossy(&again).contains("1 already in"));
        Ok(())
    }

    #[test]
    fn an_unreadable_output_path_is_an_error() -> Result<(), DistillError> {
        let folder = Folder::new("unreadable")?;
        let mut arguments = folder.arguments(&["--dry-run"]);

        if let Some(out) = arguments.get_mut(4) {
            *out = folder.path.display().to_string();
        }
        let mut out = Vec::new();
        assert!(matches!(
            main_with(&arguments, None, &mut out),
            Err(DistillError::Io { .. })
        ));
        Ok(())
    }
}
