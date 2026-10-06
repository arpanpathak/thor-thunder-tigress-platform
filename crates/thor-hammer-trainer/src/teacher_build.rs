//! The `teacher` command: checks the teacher's conversations and queues real
//! sections to write new ones from.
//!
//! ```text
//! teacher [check] [SOURCE_DIR] [OUTPUT_DIR]   check every entry, write the outputs
//! teacher pick COUNT [SOURCE...]              queue COUNT real sections per source
//! ```
//!
//! `check` runs [`crate::verify`] on every entry of `train/teacher/**/*.md`.
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
    fmt::Write as _,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    thread,
};

use serde::Serialize;

use crate::{
    error::DataError,
    pick::{self, Section},
    report::CHARS_PER_TOKEN,
    teacher::{self, Conversation, Role, Turn},
    verify::{self, Checked},
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
pub const USAGE: &str = "usage: teacher [check] [SOURCE_DIR] [OUTPUT_DIR]\n       teacher pick COUNT [SOURCE...]";

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
        Self { source, output, corpus: PathBuf::from(CORPUS), manifest: PathBuf::from(MANIFEST) }
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
            return Command::Pick { count, sources: arguments.collect() };
        }
        arguments.next_if(|first| first == "check");
        let source = PathBuf::from(arguments.next().unwrap_or_else(|| DEFAULT_SOURCE.to_string()));
        let output = PathBuf::from(arguments.next().unwrap_or_else(|| DEFAULT_OUTPUT.to_string()));
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
    let teachable = sections.iter().filter(|section| pick::is_teachable(section)).count();
    let mut summary = format!("{} sections queued in {} ({teachable} teachable, {} already used)\n", picked.len(), path.display(), used.len());
    for (source, count) in per_source {
        let _ = writeln!(summary, "  {source:<44} {count}");
    }
    Ok(summary)
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
        self.report.split("\n## Problems").next().unwrap_or_default()
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
    let needs_sections = entries.iter().any(|(_, parsed)| parsed.as_ref().is_ok_and(|entry| entry.section.is_some()));
    let sections: HashMap<String, Section> = if needs_sections {
        paths.sections()?.into_iter().map(|section| (section.id.clone(), section)).collect()
    } else {
        HashMap::new()
    };
    let entries = entries.into_iter().map(|(origin, parsed)| {
        let known = parsed.and_then(|entry| match &entry.section {
            Some(id) if !sections.contains_key(id) => Err(format!("section {id} is not in {TRAINING_FILE} or the code repositories")),
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
    let passed: Vec<(&Conversation, &Checked)> = outcomes.iter().filter_map(Outcome::passed).collect();
    write_jsonl(
        &output.join("teacher.jsonl"),
        passed.iter().map(|(conversation, checked)| chat_line(conversation, checked, &sections)),
    )?;
    write_jsonl(
        &output.join("teacher_preferences.jsonl"),
        passed.iter().filter_map(|(conversation, _)| preference_line(conversation)),
    )?;
    let report = render_report(&outcomes, &sections);
    let report_path = output.join("teacher.md");
    fs::write(&report_path, &report).map_err(DataError::io(&report_path))?;
    Ok(CheckResult { report, all_passed: passed.len() == outcomes.len() })
}

fn markdown_files(folder: &Path, files: &mut Vec<PathBuf>) -> Result<(), DataError> {
    for entry in fs::read_dir(folder).map_err(DataError::io(folder))?.filter_map(Result::ok) {
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
        let name = file.strip_prefix(source).unwrap_or(&file).display().to_string();
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
    let chunks: Vec<Vec<Entry>> = entries.chunks(chunk_size).map(<[_]>::to_vec).collect();
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

fn chat_line<'a>(conversation: &'a Conversation, checked: &'a Checked, sections: &'a HashMap<String, Section>) -> ChatLine<'a> {
    let section = conversation.section.as_ref().and_then(|id| sections.get(id));
    ChatLine {
        id: conversation.id(),
        source: &conversation.source,
        origin: &conversation.origin,
        section: conversation.section.as_deref(),
        licence: conversation.licence.as_deref().or(section.map(|found| found.licence.as_str())),
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

fn render_report(outcomes: &[Outcome], sections: &HashMap<String, Section>) -> String {
    let passed: Vec<(&Conversation, &Checked)> = outcomes.iter().filter_map(Outcome::passed).collect();
    let turns: usize = passed.iter().map(|(conversation, _)| conversation.turns.len()).sum();
    let multi_turn = passed.iter().filter(|(conversation, _)| conversation.turns.len() > 2).count();
    let grounded = passed.iter().filter(|(conversation, _)| conversation.section.is_some()).count();
    let tests: usize = passed.iter().map(|(_, checked)| checked.tests).sum();
    let runs: usize = passed.iter().map(|(_, checked)| checked.runs).sum();
    let notes: usize = passed.iter().map(|(_, checked)| checked.notes.len()).sum();
    let ignored: usize = passed.iter().map(|(_, checked)| checked.ignored).sum();
    let tokens: usize = passed.iter().map(|(conversation, _)| conversation.char_count()).sum::<usize>() / CHARS_PER_TOKEN;
    let rejected: Vec<&String> = passed.iter().filter_map(|(conversation, _)| conversation.rejected.as_ref()).collect();
    let caught = rejected.iter().filter(|text| !verify::spark_objections(text).is_empty()).count();
    let mut languages: BTreeMap<&str, usize> = BTreeMap::new();
    let mut sources: BTreeMap<&str, usize> = BTreeMap::new();
    for (conversation, checked) in &passed {
        for (language, count) in &checked.blocks {
            *languages.entry(language).or_insert(0) += count;
        }
        let source = conversation.section.as_ref().and_then(|id| sections.get(id)).map_or("written by the teacher", |found| found.source.as_str());
        *sources.entry(source).or_insert(0) += 1;
    }
    let mut report = format!(
        "# Teacher set\n\n\
         | | |\n|---|---|\n\
         | Entries | {} |\n| Passed every check | {} |\n| Written from a real section | {grounded} |\n\
         | Multi-turn conversations | {multi_turn} |\n| Turns | {turns} |\n\
         | Rust tests run and passed | {tests} |\n| Programs in other languages run cleanly | {runs} |\n\
         | Fragments marked `ignore`, not built | {ignored} |\n\
         | Broken Rust rules kept as notes (grounded entries) | {notes} |\n\
         | Preference pairs | {} |\n| Rejected answers spark also catches | {caught} of {} |\n| Tokens (estimate) | {tokens} |\n",
        outcomes.len(),
        passed.len(),
        rejected.len(),
        rejected.len(),
    );
    report.push_str("\n## Code blocks built, by language\n\n| Language | Blocks |\n|---|---|\n");
    for (language, count) in languages {
        let _ = writeln!(report, "| {language} | {count} |");
    }
    report.push_str("\n## Conversations by source\n\n| Source | Conversations |\n|---|---|\n");
    for (source, count) in sources {
        let _ = writeln!(report, "| {source} | {count} |");
    }
    report.push_str("\n## Problems\n\n");
    let failed: Vec<&Outcome> = outcomes.iter().filter(|outcome| outcome.passed().is_none()).collect();
    if failed.is_empty() {
        report.push_str("- none\n");
    }
    for outcome in failed {
        let _ = write!(report, "### {}\n\n```text\n{}\n```\n\n", outcome.origin, outcome.problems().join("\n"));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Workspace {
        root: PathBuf,
        paths: Paths,
    }

    impl Workspace {
        fn new(name: &str) -> Result<Self, DataError> {
            let root = std::env::temp_dir().join(format!("thor-hammer-teacher-{name}-{}", std::process::id()));
            let paths = Paths {
                source: root.join("teacher"),
                output: root.join("data"),
                corpus: root.join("corpus"),
                manifest: root.join("manifest.tsv"),
            };
            let workspace = Self { root, paths };
            workspace.write("manifest.tsv", "source\tkind\tcommit\tlicence_file\tlicence\nbook\tbook\tabc\tLICENSE\tMIT License\nlib\tcode\tabc\tLICENSE\tMIT License\n")?;
            let train = r#"{"id":"s1","instruction":"","response":"A long passage about heaps.","source":"corpus","origin":"book/heaps.md"}"#;
            workspace.write("data/train.jsonl", train)?;
            workspace.write("corpus/lib/src/heap.py", &"def push(heap, item):\n    heap.append(item)\n".repeat(60))?;
            Ok(workspace)
        }

        fn write(&self, path: &str, text: &str) -> Result<(), DataError> {
            let file = self.root.join(path);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent).map_err(DataError::io(parent))?;
            }
            fs::write(&file, text).map_err(DataError::io(&file))
        }

        fn read(&self, path: &str) -> Result<String, DataError> {
            let file = self.root.join(path);
            fs::read_to_string(&file).map_err(DataError::io(&file))
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    const GOOD: &str = "<!-- source: book/heaps.md; section: s1; licence: MIT -->\n### User\nWhat is a heap?\n\n### Assistant\nA tree where every parent is at least as large as its children.\n\n### Rejected\nGreat question! A heap is a robust and scalable data structure.\n";

    #[test]
    fn reads_the_command_line() {
        let words = |list: &[&str]| Command::from_args(list.iter().map(ToString::to_string));
        assert_eq!(words(&[]), Command::Check { source: DEFAULT_SOURCE.into(), output: DEFAULT_OUTPUT.into() });
        assert_eq!(words(&["check", "a", "b"]), Command::Check { source: "a".into(), output: "b".into() });
        assert_eq!(words(&["pick", "3", "trpl"]), Command::Pick { count: 3, sources: vec!["trpl".to_string()] });
        assert_eq!(words(&["pick", "many"]), Command::Usage);
    }

    #[test]
    fn writes_passing_entries_and_reports_failing_ones() -> Result<(), DataError> {
        let workspace = Workspace::new("check")?;
        workspace.write("teacher/a.md", GOOD)?;
        let unknown = GOOD.replace("section: s1", "section: nowhere");
        workspace.write("teacher/more/b.md", &format!("{unknown}---\n### User\nno source comment\n"))?;
        let result = check(&workspace.paths)?;
        assert!(!result.all_passed);
        assert!(result.summary().contains("| Passed every check | 1 |"));
        assert!(result.report.contains("section nowhere is not in"));
        assert!(result.report.contains("format: no <!-- source"));
        let chat = workspace.read("data/teacher.jsonl")?;
        assert!(chat.contains("\"source_text\":\"A long passage about heaps.\""));
        assert!(chat.contains("\"licence\":\"MIT\""));
        let preferences = workspace.read("data/teacher_preferences.jsonl")?;
        assert!(preferences.contains("\"rejected\":[{\"role\":\"assistant\",\"content\":\"Great question!"));
        assert!(result.report.contains("| book | 1 |"));
        assert!(result.report.contains("| Rejected answers spark also catches | 1 of 1 |"));
        Ok(())
    }

    #[test]
    fn a_set_without_grounded_entries_needs_no_sections() -> Result<(), DataError> {
        let workspace = Workspace::new("plain")?;
        workspace.write("teacher/a.md", "<!-- source: notes -->\n### User\nQ\n\n### Assistant\nA plain answer.\n")?;
        fs::remove_file(workspace.root.join("data/train.jsonl")).map_err(DataError::io(&workspace.root))?;
        let result = check(&workspace.paths)?;
        assert!(result.all_passed);
        assert!(result.report.contains("| written by the teacher | 1 |"));
        assert!(result.report.contains("- none"));
        Ok(())
    }

    #[test]
    fn queues_unused_prose_and_code_sections() -> Result<(), DataError> {
        let workspace = Workspace::new("queue")?;
        workspace.write("data/train.jsonl", &format!(
            r#"{{"id":"s1","instruction":"","response":"{}","source":"corpus","origin":"book/heaps.md"}}"#,
            "heaps ".repeat(200)
        ))?;
        workspace.write("teacher/a.md", "<!-- source: notes -->\n### User\nQ\n\n### Assistant\nA.\n")?;
        let summary = queue(&workspace.paths, 5, &[])?;
        assert!(summary.starts_with("2 sections queued"));
        let queued = workspace.read("data/teacher_queue.md")?;
        assert!(queued.contains("section: s1; licence: MIT"));
        assert!(queued.contains("````python\ndef push"));
        workspace.write("teacher/a.md", GOOD)?;
        assert!(queue(&workspace.paths, 5, &["book".to_string()])?.starts_with("0 sections queued"));
        Ok(())
    }

    #[test]
    fn default_paths_point_at_the_repository_layout() {
        let paths = Paths::new("t".into(), "d".into());
        assert_eq!((paths.corpus, paths.manifest), (PathBuf::from(CORPUS), PathBuf::from(MANIFEST)));
    }
}
