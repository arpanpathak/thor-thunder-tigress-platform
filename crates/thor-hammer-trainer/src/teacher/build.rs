//! The `teacher` command: checks the teacher's conversations and queues real
//! sections to write new ones from.
//!
//! ```text
//! teacher [check] [SOURCE_DIR] [OUTPUT_DIR]   check every entry, write the outputs
//! teacher pick COUNT [SOURCE...]              queue COUNT real sections per source
//! ```
//!
//! `check` runs [`crate::teacher::verify`] on every entry of `train/teacher/**/*.md`.
//! Entries that pass go to `teacher.jsonl` (chat messages, for SFT) and, when
//! they carry a rejected answer, to `teacher_preferences.jsonl` (prompt,
//! chosen, rejected, for DPO). An entry written from a real section carries
//! that section's text, file and licence, so a reviewer can compare the two.
//! `teacher.md` reports the numbers and every problem.
//!
//! `pick` writes `data/teacher_queue.md`: prose sections from
//! `data/train.jsonl` and source files from the corpus's code repositories
//! that no entry has used yet, each with the comment an entry written from it
//! starts with.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

use serde::Serialize;

use crate::{
    error::DataError,
    teacher::{
        pick::{self, Section},
        self, Conversation, Role, Turn,
        verify::{self, Checked},
    },
    report::CHARS_PER_TOKEN,
};

/// Where the entries are read from when no argument is given.
const DEFAULT_SOURCE: &str = "train/teacher";

/// Where the files are written when no argument is given.
const DEFAULT_OUTPUT: &str = "data";

/// The training file the prose sections are read from, inside the output folder.
const TRAINING_FILE: &str = "train.jsonl";

/// The corpus manifest with each source's licence.
const MANIFEST: &str = "train/corpus.manifest.tsv";

/// The folder the corpus repositories are fetched into.
const CORPUS: &str = "corpus";

/// The usage line printed for arguments that can't be read.
pub const USAGE: &str =
    "usage: teacher [check] [SOURCE_DIR] [OUTPUT_DIR]\n       teacher pick COUNT [SOURCE...]";

/// Where the command reads and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The folder of teacher entries.
    pub source: PathBuf,
    /// The folder the outputs go to; it also holds `train.jsonl`.
    pub output: PathBuf,
    /// The corpus folder with the code repositories.
    pub corpus: PathBuf,
    /// The corpus manifest.
    pub manifest: PathBuf,
}

impl Paths {
    /// The default layout, relative to the repository root, with `source`
    /// and `output` replaced.
    #[must_use]
    pub fn new(source: PathBuf, output: PathBuf) -> Self {
        Self {
            source,
            output,
            corpus: PathBuf::from(CORPUS),
            manifest: PathBuf::from(MANIFEST),
        }
    }

    /// Every section an entry can be written from: prose passages and source files.
    ///
    /// # Errors
    ///
    /// `DataError::Io` or `DataError::Json` when a file can't be read.
    pub fn sections(&self) -> Result<Vec<Section>, DataError> {
        let mut sections = pick::sections(&self.output.join(TRAINING_FILE), &self.manifest)?;
        sections.extend(pick::code_sections(&self.corpus, &self.manifest)?);

        Ok(sections)
    }
}

/// Checks running at once; each builds in its own folder.
const WORKERS: usize = 4;

/// An entry's origin, and the conversation or why it could not be read.
type Entry = (String, Result<Conversation, String>);

/// What the program was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Check every entry and write the outputs.
    Check {
        /// The folder of entries.
        source: PathBuf,
        /// The output folder.
        output: PathBuf,
    },
    /// Queue `count` unused sections per source.
    Pick {
        /// Sections per source.
        count: usize,
        /// The sources to pick from; every source when empty.
        sources: Vec<String>,
    },
    /// The arguments could not be read.
    Usage,
}

impl Command {
    /// Reads the arguments, without the program name.
    #[must_use]
    pub fn from_args(arguments: impl IntoIterator<Item = String>) -> Command {
        let mut arguments = arguments.into_iter().peekable();

        if arguments.next_if(|first| first == "pick").is_some() {
            let Some(count) = arguments.next().and_then(|count| count.parse().ok()) else {
                return Command::Usage;
            };

            return Command::Pick {
                count,
                sources: arguments.collect(),
            };
        }

        arguments.next_if(|first| first == "check");

        let source = PathBuf::from(
            arguments
                .next()
                .unwrap_or_else(|| DEFAULT_SOURCE.to_string()),
        );
        let output = PathBuf::from(
            arguments
                .next()
                .unwrap_or_else(|| DEFAULT_OUTPUT.to_string()),
        );

        Command::Check { source, output }
    }
}

/// One entry and what checking it found.
struct Outcome {
    origin: String,
    result: Result<(Conversation, Checked), String>,
}

impl Outcome {
    fn passed(&self) -> Option<(&Conversation, &Checked)> {
        match &self.result {
            Ok((conversation, checked)) if checked.problems.is_empty() => {
                Some((conversation, checked))
            }
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

/// The `source` every line of `teacher.jsonl` carries, so the review page
/// shows the teacher set as one source.
const TEACHER_SOURCE: &str = "teacher";

/// One line of `teacher.jsonl`. `origin` is the real file a grounded entry
/// was written from, or the entry itself for one the teacher wrote freely;
/// `entry` is always the file and number of the entry.
#[derive(Serialize)]
struct ChatLine<'a> {
    id: String,
    source: &'static str,
    origin: &'a str,
    entry: &'a str,
    based_on: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    section: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    licence: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_text: Option<&'a str>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    notes: &'a [String],
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

/// Writes `teacher_queue.md` in the output folder with `count` unused
/// sections per source, and returns the summary to print.
///
/// # Errors
///
/// `DataError::Io` or `DataError::Json` when an input can't be read or the
/// queue can't be written.
pub fn queue(paths: &Paths, count: usize, sources: &[String]) -> Result<String, DataError> {
    let sections = paths.sections()?;
    let used: HashSet<String> = read_entries(&paths.source)?
        .into_iter()
        .filter_map(|(_, parsed)| parsed.ok()?.section)
        .collect();
    let picked = pick::pick(&sections, count, sources, &used);
    let path = paths.output.join("teacher_queue.md");
    fs::write(&path, pick::render_queue(&picked)).map_err(DataError::io(&path))?;

    let mut per_source: BTreeMap<&str, usize> = BTreeMap::new();

    for section in &picked {
        *per_source.entry(section.source.as_str()).or_insert(0) += 1;
    }

    let teachable = sections
        .iter()
        .filter(|section| pick::is_teachable(section))
        .count();
    let per_source: String = per_source
        .iter()
        .map(|(source, count)| format!("  {source:<44} {count}\n"))
        .collect();

    Ok(format!(
        "{} sections queued in {} ({teachable} teachable, {} already used)\n{per_source}",
        picked.len(),
        path.display(),
        used.len()
    ))
}

/// What a check found: the report, and whether every entry passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    /// The text of `teacher.md`.
    pub report: String,
    /// True when every entry passed.
    pub all_passed: bool,
}

impl CheckResult {
    /// The report without its list of problems, for the terminal.
    #[must_use]
    pub fn summary(&self) -> &str {
        self.report
            .split("\n## Problems")
            .next()
            .unwrap_or_default()
    }
}

/// Checks every entry and writes the outputs.
///
/// # Errors
///
/// `DataError::Io` or `DataError::Json` when an input can't be read, a
/// toolchain can't be started, or an output can't be written.
pub fn check(paths: &Paths) -> Result<CheckResult, DataError> {
    let (source, output) = (paths.source.as_path(), paths.output.as_path());
    let entries = read_entries(source)?;
    let sections = known_sections(paths, &entries)?;
    let entries = entries.into_iter().map(|(origin, parsed)| {
        let known = parsed.and_then(|entry| match &entry.section {
            Some(id) if !sections.contains_key(id) => Err(format!(
                "section {id} is not in {TRAINING_FILE} or the code repositories"
            )),
            _ => Ok(entry),
        });

        (origin, known)
    });
    let scratch = output.join("teacher-scratch");
    let outcomes = check_all(entries.collect(), &scratch)?;

    if scratch.exists() {
        fs::remove_dir_all(&scratch).map_err(DataError::io(&scratch))?;
    }

    fs::create_dir_all(output).map_err(DataError::io(output))?;

    let report = write_outputs(output, &outcomes, &sections)?;

    Ok(CheckResult {
        report,
        all_passed: outcomes.iter().all(|outcome| outcome.passed().is_some()),
    })
}

/// The sections the entries may name, by id. They are read only when an
/// entry names one, since reading them takes the whole training file.
fn known_sections(paths: &Paths, entries: &[Entry]) -> Result<HashMap<String, Section>, DataError> {
    let needed = entries
        .iter()
        .any(|(_, parsed)| parsed.as_ref().is_ok_and(|entry| entry.section.is_some()));

    if !needed {
        return Ok(HashMap::new());
    }

    Ok(paths
        .sections()?
        .into_iter()
        .map(|section| (section.id.clone(), section))
        .collect())
}

/// Writes the training lines, the preference pairs and the report to
/// `output`, and returns the report.
fn write_outputs(
    output: &Path,
    outcomes: &[Outcome],
    sections: &HashMap<String, Section>,
) -> Result<String, DataError> {
    let passed: Vec<Passed<'_>> = outcomes.iter().filter_map(Outcome::passed).collect();
    let chat_lines = passed
        .iter()
        .map(|(conversation, checked)| chat_line(conversation, checked, sections));
    let preference_lines = passed
        .iter()
        .filter_map(|(conversation, _)| preference_line(conversation));
    let report = render_report(outcomes, sections);
    let report_path = output.join("teacher.md");

    write_jsonl(&output.join("teacher.jsonl"), chat_lines)?;
    write_jsonl(&output.join("teacher_preferences.jsonl"), preference_lines)?;
    fs::write(&report_path, &report).map_err(DataError::io(&report_path))?;

    Ok(report)
}

fn markdown_files(folder: &Path, files: &mut Vec<PathBuf>) -> Result<(), DataError> {
    for entry in fs::read_dir(folder)
        .map_err(DataError::io(folder))?
        .filter_map(Result::ok)
    {
        let path = entry.path();

        if path.is_dir() {
            markdown_files(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "md") {
            files.push(path);
        }
    }

    Ok(())
}

fn read_entries(source: &Path) -> Result<Vec<Entry>, DataError> {
    let mut files = Vec::new();
    markdown_files(source, &mut files)?;
    files.sort();

    let mut entries = Vec::new();

    for file in files {
        let markdown = fs::read_to_string(&file).map_err(DataError::io(&file))?;
        let name = file
            .strip_prefix(source)
            .unwrap_or(&file)
            .display()
            .to_string();

        entries.extend(
            teacher::entries(&markdown, &name)
                .into_iter()
                .map(|(origin, parsed)| {
                    (origin, parsed.map_err(|error| format!("format: {error}")))
                }),
        );
    }

    Ok(entries)
}

fn check_all(entries: Vec<Entry>, scratch: &Path) -> Result<Vec<Outcome>, DataError> {
    let chunk_size = entries.len().div_ceil(WORKERS).max(1);
    let chunks: Vec<Vec<Entry>> = entries.chunks(chunk_size).map(<[_]>::to_vec).collect();
    let (sender, receiver) = mpsc::channel();

    thread::scope(|scope| {
        for (worker, chunk) in chunks.into_iter().enumerate() {
            let sender = sender.clone();
            let folder = scratch.join(worker.to_string());
            scope.spawn(move || sender.send((worker, check_chunk(chunk, &folder))));
        }
    });

    drop(sender);

    let mut finished: Vec<(usize, Result<Vec<Outcome>, DataError>)> =
        receiver.into_iter().collect();
    finished.sort_by_key(|&(worker, _)| worker);

    let mut outcomes = Vec::new();

    for (_, checked) in finished {
        outcomes.extend(checked?);
    }

    Ok(outcomes)
}

fn check_chunk(chunk: Vec<Entry>, folder: &Path) -> Result<Vec<Outcome>, DataError> {
    chunk
        .into_iter()
        .map(|(origin, parsed)| {
            let result = match parsed {
                Ok(conversation) => verify::check(&conversation, folder)
                    .map(|checked| Ok((conversation, checked)))?,
                Err(format) => Err(format),
            };

            Ok(Outcome { origin, result })
        })
        .collect()
}

fn chat_line<'a>(
    conversation: &'a Conversation,
    checked: &'a Checked,
    sections: &'a HashMap<String, Section>,
) -> ChatLine<'a> {
    let section = conversation
        .section
        .as_ref()
        .and_then(|id| sections.get(id));

    ChatLine {
        id: conversation.id(),
        source: TEACHER_SOURCE,
        origin: section.map_or(conversation.origin.as_str(), |found| found.origin.as_str()),
        entry: &conversation.origin,
        based_on: &conversation.source,
        section: conversation.section.as_deref(),
        licence: conversation
            .licence
            .as_deref()
            .or(section.map(|found| found.licence.as_str())),
        source_text: section.map(|found| found.text.as_str()),
        notes: &checked.notes,
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
        rejected: [Turn {
            role: Role::Assistant,
            content: rejected.clone(),
        }],
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

/// An entry that passed, with what its checks found.
type Passed<'a> = (&'a Conversation, &'a Checked);

/// What the entries that passed add up to.
struct Totals {
    turns: usize,
    multi_turn: usize,
    grounded: usize,
    tests: usize,
    runs: usize,
    notes: usize,
    ignored: usize,
    tokens: usize,
    pairs: usize,
    caught: usize,
}

impl Totals {
    fn of(passed: &[Passed<'_>]) -> Self {
        let total = |of: fn(&Checked) -> usize| -> usize {
            passed.iter().map(|(_, checked)| of(checked)).sum()
        };
        let count = |is: fn(&Conversation) -> bool| -> usize {
            passed.iter().filter(|(spoken, _)| is(spoken)).count()
        };
        let chars: usize = passed.iter().map(|(spoken, _)| spoken.char_count()).sum();
        let rejected: Vec<&String> = passed
            .iter()
            .filter_map(|(spoken, _)| spoken.rejected.as_ref())
            .collect();

        Totals {
            turns: passed.iter().map(|(spoken, _)| spoken.turns.len()).sum(),
            multi_turn: count(|spoken| spoken.turns.len() > 2),
            grounded: count(|spoken| spoken.section.is_some()),
            tests: total(|checked| checked.tests),
            runs: total(|checked| checked.runs),
            notes: total(|checked| checked.notes.len()),
            ignored: total(|checked| checked.ignored),
            tokens: chars / CHARS_PER_TOKEN,
            pairs: rejected.len(),
            caught: rejected
                .iter()
                .filter(|text| !verify::spark_objections(text).is_empty())
                .count(),
        }
    }

    /// The report's first table, for `entries` entries of which `passed` passed.
    fn table(&self, entries: usize, passed: usize) -> String {
        let Totals {
            turns,
            multi_turn,
            grounded,
            tests,
            runs,
            notes,
            ignored,
            tokens,
            pairs,
            caught,
        } = self;

        format!(
            "# Teacher set\n\n\
             | | |\n|---|---|\n\
             | Entries | {entries} |\n| Passed every check | {passed} |\n| Written from a real section | {grounded} |\n\
             | Multi-turn conversations | {multi_turn} |\n| Turns | {turns} |\n\
             | Rust tests run and passed | {tests} |\n| Programs in other languages run cleanly | {runs} |\n\
             | Fragments marked `ignore`, not built | {ignored} |\n\
             | Broken Rust rules kept as notes (grounded entries) | {notes} |\n\
             | Preference pairs | {pairs} |\n| Rejected answers spark also catches | {caught} of {pairs} |\n| Tokens (estimate) | {tokens} |\n"
        )
    }
}

/// How many code blocks were built in each language.
fn blocks_by_language<'a>(passed: &[Passed<'a>]) -> BTreeMap<&'a str, usize> {
    let mut languages = BTreeMap::new();

    for (language, count) in passed.iter().flat_map(|(_, checked)| &checked.blocks) {
        *languages.entry(language.as_ref()).or_insert(0) += count;
    }

    languages
}

/// How many conversations were written from each source.
fn conversations_by_source<'a>(
    passed: &[Passed<'a>],
    sections: &'a HashMap<String, Section>,
) -> BTreeMap<&'a str, usize> {
    let mut sources = BTreeMap::new();

    for (conversation, _) in passed {
        let source = conversation
            .section
            .as_ref()
            .and_then(|id| sections.get(id))
            .map_or("written by the teacher", |found| found.source.as_str());
        *sources.entry(source).or_insert(0) += 1;
    }

    sources
}

/// A two-column table under `heading`, one row per count.
fn counts_table(heading: &str, columns: &str, counts: &BTreeMap<&str, usize>) -> String {
    let rows: String = counts
        .iter()
        .map(|(name, count)| format!("| {name} | {count} |\n"))
        .collect();

    format!("\n## {heading}\n\n| {columns} |\n|---|---|\n{rows}")
}

/// The entries that failed, each with what was wrong with it.
fn problems(outcomes: &[Outcome]) -> String {
    let failed: String = outcomes
        .iter()
        .filter(|outcome| outcome.passed().is_none())
        .map(|outcome| {
            let found = outcome.problems().join("\n");

            format!("### {}\n\n```text\n{found}\n```\n\n", outcome.origin)
        })
        .collect();
    let listed = if failed.is_empty() {
        "- none\n"
    } else {
        &failed
    };

    format!("\n## Problems\n\n{listed}")
}

fn render_report(outcomes: &[Outcome], sections: &HashMap<String, Section>) -> String {
    let passed: Vec<Passed<'_>> = outcomes.iter().filter_map(Outcome::passed).collect();
    let languages = blocks_by_language(&passed);
    let sources = conversations_by_source(&passed, sections);

    [
        Totals::of(&passed).table(outcomes.len(), passed.len()),
        counts_table(
            "Code blocks built, by language",
            "Language | Blocks",
            &languages,
        ),
        counts_table(
            "Conversations by source",
            "Source | Conversations",
            &sources,
        ),
        problems(outcomes),
    ]
    .concat()
}

#[cfg(test)]
mod tests;
