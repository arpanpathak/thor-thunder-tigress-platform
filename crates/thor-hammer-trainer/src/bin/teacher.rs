//! Builds the teacher's training data from `train/teacher/*.md`.
//!
//! ```text
//! cargo run --release -p thor-hammer-trainer --bin teacher -- [SOURCE_DIR] [OUTPUT_DIR]
//! ```
//!
//! Every entry is checked by [`thor_hammer_trainer::verify`]. Entries that pass
//! go to `teacher.jsonl` (chat messages, for SFT) and, when they carry a
//! rejected answer, to `teacher_preferences.jsonl` (prompt, chosen, rejected,
//! for DPO). `teacher.md` reports the numbers and every problem. The program
//! fails when any entry fails, after writing the ones that pass.

use std::{
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    thread,
};

use serde::Serialize;
use thor_hammer_trainer::{
    error::DataError,
    report::CHARS_PER_TOKEN,
    teacher::{self, Conversation, Role, Turn},
    verify::{self, Checked},
};

/// Where the entries are read from when no argument is given.
const DEFAULT_SOURCE: &str = "train/teacher";

/// Where the files are written when no argument is given.
const DEFAULT_OUTPUT: &str = "data";

/// Checks running at once; each builds in its own folder.
const WORKERS: usize = 4;

/// An entry's origin, and the conversation or why it could not be read.
type Entry = (String, Result<Conversation, String>);

/// One entry and what checking it found.
struct Outcome {
    origin: String,
    result: Result<(Conversation, Checked), String>,
}

impl Outcome {
    fn passed(&self) -> Option<(&Conversation, &Checked)> {
        match &self.result {
            Ok((conversation, checked)) if checked.problems.is_empty() => Some((conversation, checked)),
            Ok(_) | Err(_) => None,
        }
    }

    fn problems(&self) -> Vec<String> {
        match &self.result {
            Ok((_, checked)) => checked.problems.iter().map(ToString::to_string).collect(),
            Err(format) => vec![format.clone()],
        }
    }
}

/// One line of `teacher.jsonl`.
#[derive(Serialize)]
struct ChatLine<'a> {
    id: String,
    source: &'a str,
    origin: &'a str,
    messages: &'a [Turn],
}

/// One line of `teacher_preferences.jsonl`, in TRL's conversational format.
#[derive(Serialize)]
struct PreferenceLine<'a> {
    id: String,
    source: &'a str,
    origin: &'a str,
    prompt: &'a [Turn],
    chosen: [&'a Turn; 1],
    rejected: [Turn; 1],
}

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let source = PathBuf::from(arguments.next().unwrap_or_else(|| DEFAULT_SOURCE.to_string()));
    let output = PathBuf::from(arguments.next().unwrap_or_else(|| DEFAULT_OUTPUT.to_string()));
    match build(&source, &output) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("teacher: some entries failed; see {}", output.join("teacher.md").display());
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("teacher: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Checks every entry and writes the outputs. True when every entry passed.
fn build(source: &Path, output: &Path) -> Result<bool, DataError> {
    let entries = read_entries(source)?;
    let scratch = output.join("teacher-scratch");
    let outcomes = check_all(entries, &scratch)?;
    fs::remove_dir_all(&scratch).map_err(DataError::io(&scratch))?;
    fs::create_dir_all(output).map_err(DataError::io(output))?;
    let passed: Vec<(&Conversation, &Checked)> = outcomes.iter().filter_map(Outcome::passed).collect();
    write_jsonl(&output.join("teacher.jsonl"), passed.iter().map(|(conversation, _)| chat_line(conversation)))?;
    write_jsonl(
        &output.join("teacher_preferences.jsonl"),
        passed.iter().filter_map(|(conversation, _)| preference_line(conversation)),
    )?;
    let report = render_report(&outcomes);
    let report_path = output.join("teacher.md");
    fs::write(&report_path, &report).map_err(DataError::io(&report_path))?;
    print!("{}", report.split("\n## Problems").next().unwrap_or_default());
    Ok(passed.len() == outcomes.len())
}

fn read_entries(source: &Path) -> Result<Vec<Entry>, DataError> {
    let mut files: Vec<PathBuf> = fs::read_dir(source)
        .map_err(DataError::io(source))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect();
    files.sort();
    let mut entries = Vec::new();
    for file in files {
        let markdown = fs::read_to_string(&file).map_err(DataError::io(&file))?;
        let name = file.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        entries.extend(
            teacher::entries(&markdown, &name)
                .into_iter()
                .map(|(origin, parsed)| (origin, parsed.map_err(|error| format!("format: {error}")))),
        );
    }
    Ok(entries)
}

fn check_all(entries: Vec<Entry>, scratch: &Path) -> Result<Vec<Outcome>, DataError> {
    let chunk_size = entries.len().div_ceil(WORKERS).max(1);
    let chunks: Vec<Vec<Entry>> =
        entries.chunks(chunk_size).map(<[_]>::to_vec).collect();
    thread::scope(|scope| {
        let workers: Vec<_> = chunks
            .into_iter()
            .enumerate()
            .map(|(worker, chunk)| {
                let folder = scratch.join(worker.to_string());
                scope.spawn(move || check_chunk(chunk, &folder))
            })
            .collect();
        let mut outcomes = Vec::new();
        for worker in workers {
            let checked = worker.join().map_err(|_| DataError::Io {
                path: scratch.display().to_string(),
                source: std::io::Error::other("a checking thread panicked"),
            })??;
            outcomes.extend(checked);
        }
        Ok(outcomes)
    })
}

fn check_chunk(chunk: Vec<Entry>, folder: &Path) -> Result<Vec<Outcome>, DataError> {
    chunk
        .into_iter()
        .map(|(origin, parsed)| {
            let result = match parsed {
                Ok(conversation) => verify::check(&conversation, folder).map(|checked| Ok((conversation, checked)))?,
                Err(format) => Err(format),
            };
            Ok(Outcome { origin, result })
        })
        .collect()
}

fn chat_line(conversation: &Conversation) -> ChatLine<'_> {
    ChatLine {
        id: conversation.id(),
        source: &conversation.source,
        origin: &conversation.origin,
        messages: &conversation.turns,
    }
}

fn preference_line(conversation: &Conversation) -> Option<PreferenceLine<'_>> {
    let rejected = conversation.rejected.as_ref()?;
    let chosen = conversation.last_answer()?;
    Some(PreferenceLine {
        id: conversation.id(),
        source: &conversation.source,
        origin: &conversation.origin,
        prompt: conversation.prompt(),
        chosen: [chosen],
        rejected: [Turn { role: Role::Assistant, content: rejected.clone() }],
    })
}

fn write_jsonl<T: Serialize>(path: &Path, lines: impl Iterator<Item = T>) -> Result<(), DataError> {
    let file = fs::File::create(path).map_err(DataError::io(path))?;
    let mut writer = BufWriter::new(file);
    for line in lines {
        serde_json::to_writer(&mut writer, &line)?;
        writer.write_all(b"\n").map_err(DataError::io(path))?;
    }
    writer.flush().map_err(DataError::io(path))
}

fn render_report(outcomes: &[Outcome]) -> String {
    let passed: Vec<(&Conversation, &Checked)> = outcomes.iter().filter_map(Outcome::passed).collect();
    let turns: usize = passed.iter().map(|(conversation, _)| conversation.turns.len()).sum();
    let multi_turn = passed.iter().filter(|(conversation, _)| conversation.turns.len() > 2).count();
    let blocks: usize = passed.iter().map(|(_, checked)| checked.blocks).sum();
    let tests: usize = passed.iter().map(|(_, checked)| checked.tests).sum();
    let tokens: usize = passed.iter().map(|(conversation, _)| conversation.char_count()).sum::<usize>() / CHARS_PER_TOKEN;
    let rejected: Vec<&String> = passed.iter().filter_map(|(conversation, _)| conversation.rejected.as_ref()).collect();
    let caught = rejected.iter().filter(|text| !verify::spark_objections(text).is_empty()).count();
    let mut report = format!(
        "# Teacher set\n\n\
         | | |\n|---|---|\n\
         | Entries | {} |\n| Passed every check | {} |\n| Multi-turn conversations | {multi_turn} |\n\
         | Turns | {turns} |\n| Rust blocks built with clippy pedantic | {blocks} |\n| Tests run and passed | {tests} |\n\
         | Preference pairs | {} |\n| Rejected answers spark also catches | {caught} of {} |\n| Tokens (estimate) | {tokens} |\n",
        outcomes.len(),
        passed.len(),
        rejected.len(),
        rejected.len(),
    );
    report.push_str("\n## Problems\n\n");
    let failed: Vec<&Outcome> = outcomes.iter().filter(|outcome| outcome.passed().is_none()).collect();
    if failed.is_empty() {
        report.push_str("- none\n");
    }
    for outcome in failed {
        report.push_str(&format!("### {}\n\n```text\n{}\n```\n\n", outcome.origin, outcome.problems().join("\n")));
    }
    report
}
