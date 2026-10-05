//! Builds the instruction/response training set for fine-tuning.
//!
//! ## Inputs
//!
//! * The hand-written readability set and the Claude chat export, both in
//!   [`DATASTORE`].
//! * The chapters and source files of the book repository, [`BOOK_REPOSITORY`].
//!
//! ## Output
//!
//! * `train.jsonl`: one example per line, `{instruction, response, source, origin}`.
//! * `stats.md`: examples and estimated tokens per source, and what was left out.
//!
//! ## Pipeline
//!
//! ```text
//!   readability.md ─┐
//!   conversations ──┼─► examples ─► drop short and duplicate ─► train.jsonl
//!   book repository ┘                                        └► stats.md
//! ```
//!
//! ```text
//! cargo run --release -p thor-hammer-trainer -- [OUTPUT_DIR]
//! ```

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    book, chat, clever_vs_readable, code, corpus,
    error::DataError,
    example::{Example, SkipReason},
    readability, report, report::ReportInput,
    slop_flags,
};

/// The folder holding the chat export and the readability set.
const DATASTORE: &str = "/home/jetson/Projects/edgechat/convo_datastore";

/// The book repository named in `other_project_in_this_machine.md`.
const BOOK_REPOSITORY: &str = "/home/jetson/Projects/nvidia-cloud-software-engineer-interview";

/// The verified clever-vs-readable set, extracted from `files.zip` into the datastore.
const CLEVER_VS_READABLE_SFT: &str = "clever_vs_readable/clever_vs_readable_sft.jsonl";

/// The preference pairs of the same set.
const CLEVER_VS_READABLE_DPO: &str = "clever_vs_readable/clever_vs_readable_dpo.jsonl";

/// Slop flags set on the review page. Outside `data/` because it is human work.
const SLOP_FLAGS_FILE: &str = "labels/slop_flags.jsonl";

/// Examples shorter than this, instruction and response together, carry too
/// little to learn from.
const MIN_EXAMPLE_CHARS: usize = 200;

/// Build output, dependencies, rendered animation frames, and drafts that
/// still need human review.
const IGNORED_DIRECTORIES: [&str; 5] = [
    "target",
    "node_modules",
    "frames",
    "out",
    "__absolute__garbage_human_review_needed",
];

/// Markdown files about the books rather than book content. The anti-slop
/// list is here because training on it would teach the very phrases it bans.
const NOTES_FILES: [&str; 12] = [
    "README.md",
    "SUMMARY.md",
    "cover.md",
    "about-the-cover.md",
    "WORKLOG.md",
    "WRITING_GUIDE.md",
    "ANIMATIONS.md",
    "TODO.md",
    "START_HERE.md",
    "draft_frustration.md",
    "job-description.md",
    "anti_ai_slop.md",
];

/// What a path in the book repository holds, and so what to do with it.
enum CorpusFile {
    /// A folder to walk into.
    Directory,
    /// A markdown chapter: one example per section.
    Chapter,
    /// A markdown file of notes: left out.
    Notes,
    /// A source file: one example if it has a header comment.
    Code,
    /// Anything else, including hidden files and ignored folders.
    Ignored,
}

/// One line of `train.jsonl`: the example and its stable id, which the review
/// page uses to attach slop flags.
#[derive(Serialize)]
struct TrainingRecord<'a> {
    id: String,
    #[serde(flatten)]
    example: &'a Example,
}

impl<'a> TrainingRecord<'a> {
    /// The record for `example`, with its id computed from its text.
    fn new(example: &'a Example) -> Self {
        TrainingRecord {
            id: example.id(),
            example,
        }
    }
}

/// Reads every input, filters the examples, and writes the output files.
pub fn build_training_set(output_dir: &Path) -> Result<(), DataError> {
    let mut skip_reasons = Vec::new();
    let examples = read_all_examples(&mut skip_reasons)?;
    let unique_examples = remove_short_and_duplicate(examples, &mut skip_reasons);
    let slop_flags_by_id = slop_flags::read(Path::new(SLOP_FLAGS_FILE))?;
    let (training_set, flagged_examples, unmatched_flags) =
        slop_flags::separate_flagged(unique_examples, &slop_flags_by_id, &mut skip_reasons);
    for unmatched in &unmatched_flags {
        eprintln!(
            "warning: flag {} matched no example and was NOT applied: {}",
            unmatched.id, unmatched.note
        );
    }
    let preference_file = Path::new(DATASTORE).join(CLEVER_VS_READABLE_DPO);
    let preference_pairs = clever_vs_readable::preference_pairs(&read_file(&preference_file)?)?;
    let slop_span_counts = slop_flags::span_counts(&flagged_examples);
    let report = report::render(&ReportInput {
        training_set: &training_set,
        skip_reasons: &skip_reasons,
        preference_pair_count: preference_pairs.len(),
        slop_span_counts: &slop_span_counts,
        unmatched_flags: &unmatched_flags,
    });

    let training_records: Vec<TrainingRecord> = training_set
        .iter()
        .map(TrainingRecord::new)
        .collect();
    fs::create_dir_all(output_dir).map_err(DataError::io(output_dir))?;
    write_jsonl(&output_dir.join("train.jsonl"), &training_records)?;
    write_jsonl(&output_dir.join("slop.jsonl"), &flagged_examples)?;
    write_jsonl(&output_dir.join("preferences.jsonl"), &preference_pairs)?;
    write_file(&output_dir.join("stats.md"), &report)?;
    print!("{report}");
    Ok(())
}

/// Reads every source, curated sets first, so the curated copy of a duplicate is the one kept.
fn read_all_examples(skip_reasons: &mut Vec<SkipReason>) -> Result<Vec<Example>, DataError> {
    let datastore = Path::new(DATASTORE);
    let readability_set = read_file(&datastore.join("readability_training.md"))?;
    let clever_vs_readable_set = read_file(&datastore.join(CLEVER_VS_READABLE_SFT))?;
    let chat_export = read_file(&datastore.join("work/extracted/conversations.json"))?;
    let single_chat = read_file(&datastore.join("chat_0.md"))?;

    let mut examples = readability::examples(&readability_set);
    examples.extend(clever_vs_readable::examples(
        &clever_vs_readable_set,
        skip_reasons,
    )?);
    examples.extend(chat::examples(&chat_export, skip_reasons)?);
    examples.extend(chat::example_from_question_file(&single_chat, "chat_0.md"));
    examples.extend(book_repository_examples(
        Path::new(BOOK_REPOSITORY),
        skip_reasons,
    )?);
    examples.extend(open_corpus_examples()?);
    Ok(examples)
}

/// Reads the open corpus fetched by `train/fetch_corpus.sh`.
///
/// A source whose licence is not permissive is refused by [`corpus::examples`]
/// and reported here rather than quietly dropped.
fn open_corpus_examples() -> Result<Vec<Example>, DataError> {
    let fetched = corpus::examples(
        Path::new(corpus::CORPUS_DIRECTORY),
        Path::new(corpus::MANIFEST_FILE),
    )?;
    println!("open corpus:");
    for report in &fetched.reports {
        match &report.outcome {
            corpus::SourceOutcome::Used {
                examples,
                before_cap,
                licence,
            } => println!(
                "  {:<22} {:>6} examples  {}{}",
                report.name,
                examples,
                licence.name(),
                match examples < before_cap {
                    true => format!("  (capped from {before_cap})"),
                    false => String::new(),
                }
            ),
            corpus::SourceOutcome::LicenceRefused(licence) => println!(
                "  {:<22} refused, licence {}",
                report.name,
                licence.name()
            ),
            corpus::SourceOutcome::NoMarkdown => {
                println!("  {:<22} no markdown found", report.name);
            }
        }
    }
    Ok(fetched.examples)
}

/// Reads a whole UTF-8 file.
fn read_file(path: &Path) -> Result<String, DataError> {
    fs::read_to_string(path).map_err(DataError::io(path))
}

/// Writes a whole file, replacing it if it exists.
fn write_file(path: &Path, contents: &str) -> Result<(), DataError> {
    fs::write(path, contents).map_err(DataError::io(path))
}

/// Turns every chapter and source file under `root` into examples.
fn book_repository_examples(
    root: &Path,
    skip_reasons: &mut Vec<SkipReason>,
) -> Result<Vec<Example>, DataError> {
    let mut corpus_files = Vec::new();
    collect_corpus_files(root, &mut corpus_files)?;
    corpus_files.sort();

    let mut examples = Vec::new();
    for path in corpus_files {
        let origin = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        match classify(&path) {
            CorpusFile::Chapter => {
                examples.extend(book::examples(&corpus::clean(&read_file(&path)?), &origin))
            }
            CorpusFile::Code => match code::example(&read_file(&path)?, &path, &origin) {
                Some(example) => examples.push(example),
                None => skip_reasons.push(SkipReason::NoHeaderComment),
            },
            CorpusFile::Notes => skip_reasons.push(SkipReason::NotesFile),
            CorpusFile::Directory | CorpusFile::Ignored => {}
        }
    }
    Ok(examples)
}

/// Walks `directory` and collects every chapter, notes file and source file.
fn collect_corpus_files(
    directory: &Path,
    corpus_files: &mut Vec<PathBuf>,
) -> Result<(), DataError> {
    for entry in fs::read_dir(directory).map_err(DataError::io(directory))? {
        let path = entry.map_err(DataError::io(directory))?.path();
        match classify(&path) {
            CorpusFile::Directory => collect_corpus_files(&path, corpus_files)?,
            CorpusFile::Chapter | CorpusFile::Notes | CorpusFile::Code => corpus_files.push(path),
            CorpusFile::Ignored => {}
        }
    }
    Ok(())
}

/// Decides what a path holds from its name, extension and type.
fn classify(path: &Path) -> CorpusFile {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let is_hidden = name.starts_with('.');
    let is_ignored_directory = IGNORED_DIRECTORIES.contains(&name);
    let is_markdown = path.extension().is_some_and(|extension| extension == "md");
    match (
        is_hidden || is_ignored_directory,
        path.is_dir(),
        is_markdown,
    ) {
        (true, ..) => CorpusFile::Ignored,
        (false, true, ..) => CorpusFile::Directory,
        (false, false, true) if NOTES_FILES.contains(&name) => CorpusFile::Notes,
        (false, false, true) => CorpusFile::Chapter,
        (false, false, false) if code::is_source_file(path) => CorpusFile::Code,
        (false, false, false) => CorpusFile::Ignored,
    }
}

/// Keeps the first copy of every example that is long enough, and records why
/// each other example was dropped.
fn remove_short_and_duplicate(
    examples: Vec<Example>,
    skip_reasons: &mut Vec<SkipReason>,
) -> Vec<Example> {
    let mut seen_texts = HashSet::new();
    let mut training_set = Vec::new();
    for example in examples {
        match rejection(&example, &mut seen_texts) {
            Some(reason) => skip_reasons.push(reason),
            None => training_set.push(example),
        }
    }
    training_set
}

/// Why `example` cannot be kept, or `None` when it can. A kept example's text
/// is added to `seen_texts`, so a later copy counts as a duplicate.
fn rejection(example: &Example, seen_texts: &mut HashSet<String>) -> Option<SkipReason> {
    if example.char_count() < MIN_EXAMPLE_CHARS {
        return Some(SkipReason::TooShort);
    }
    let is_first_copy = seen_texts.insert(example.dedup_key());
    match is_first_copy {
        true => None,
        false => Some(SkipReason::Duplicate),
    }
}

/// Writes one JSON object per line.
fn write_jsonl<Record: Serialize>(path: &Path, records: &[Record]) -> Result<(), DataError> {
    let mut jsonl = String::new();
    for record in records {
        jsonl.push_str(&serde_json::to_string(record)?);
        jsonl.push('\n');
    }
    write_file(path, &jsonl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::example::Source;

    fn chat_example(response: &str) -> Example {
        Example {
            instruction: "What is a page?".to_string(),
            response: response.to_string(),
            source: Source::Chat,
            origin: String::new(),
        }
    }

    #[test]
    fn removes_short_and_duplicate_examples() {
        let long_answer = "A page is a fixed-size block of virtual memory. ".repeat(5);
        let examples = vec![
            chat_example(&long_answer),
            chat_example(&long_answer),
            chat_example("Too short."),
        ];
        let mut skip_reasons = Vec::new();
        let training_set = remove_short_and_duplicate(examples, &mut skip_reasons);
        assert_eq!(training_set, [chat_example(&long_answer)]);
        assert_eq!(skip_reasons, [SkipReason::Duplicate, SkipReason::TooShort]);
    }

    #[test]
    fn classifies_notes_and_chapters() {
        assert!(matches!(
            classify(Path::new("book/WORKLOG.md")),
            CorpusFile::Notes
        ));
        assert!(matches!(
            classify(Path::new("book/ch01.md")),
            CorpusFile::Chapter
        ));
        assert!(matches!(
            classify(Path::new("lab/topk.rs")),
            CorpusFile::Code
        ));
        assert!(matches!(
            classify(Path::new("lab/notes.txt")),
            CorpusFile::Ignored
        ));
    }
}
