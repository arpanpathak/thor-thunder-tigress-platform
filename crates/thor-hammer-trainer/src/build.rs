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
//!
//! [`Inputs`] names every input path, so the whole build can run on a small
//! tree of test files.

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
    readability, report,
    report::ReportInput,
    slop_flags,
};

/// The folder holding the chat export and the readability set, under the
/// home folder.
const DATASTORE: &str = "Projects/edgechat/convo_datastore";

/// The book repository, under the home folder.
const BOOK_REPOSITORY: &str = "Projects/nvidia-cloud-software-engineer-interview";

/// The verified clever-vs-readable set, extracted from `files.zip` into the datastore.
const CLEVER_VS_READABLE_SFT: &str = "clever_vs_readable/clever_vs_readable_sft.jsonl";

/// The preference pairs of the same set.
const CLEVER_VS_READABLE_DPO: &str = "clever_vs_readable/clever_vs_readable_dpo.jsonl";

/// Slop flags set on the review page. Outside `data/` because it is human work.
const SLOP_FLAGS_FILE: &str = "labels/slop_flags.jsonl";

/// Examples shorter than this, instruction and response together, carry too
/// little to learn from.
const MIN_EXAMPLE_CHARS: usize = 200;

/// Build output, dependencies, rendered animation frames, drafts that still
/// need human review, and the GPU Kubernetes book, which the author asked to
/// keep out of training.
const IGNORED_DIRECTORIES: [&str; 6] = [
    "target",
    "node_modules",
    "frames",
    "out",
    "__absolute__garbage_human_review_needed",
    "gpu-accelerated-kubernetes",
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
    /// A file to read.
    File(BookFile),
    /// Anything else, including hidden files and ignored folders.
    Ignored,
}

/// A file of the book repository that the build reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BookFile {
    /// A markdown chapter: one example per section.
    Chapter,
    /// A markdown file of notes: left out.
    Notes,
    /// A source file: one example if it has a header comment.
    Code,
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

/// Where the build reads from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// The folder holding the chat export, the readability set and the
    /// clever-vs-readable set.
    pub datastore: PathBuf,
    /// The book repository: chapters and source files.
    pub book_repository: PathBuf,
    /// The folder the open corpus was fetched into.
    pub corpus: PathBuf,
    /// The corpus manifest.
    pub manifest: PathBuf,
    /// The reviewer's slop flags.
    pub slop_flags: PathBuf,
}

impl Inputs {
    /// The usual layout: the datastore and book repository under `home`, the
    /// corpus and labels relative to the repository root.
    #[must_use]
    pub fn under_home(home: &Path) -> Self {
        Self {
            datastore: home.join(DATASTORE),
            book_repository: home.join(BOOK_REPOSITORY),
            corpus: PathBuf::from(corpus::CORPUS_DIRECTORY),
            manifest: PathBuf::from(corpus::MANIFEST_FILE),
            slop_flags: PathBuf::from(SLOP_FLAGS_FILE),
        }
    }
}

/// What a build wrote, for the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    /// One line per corpus source: what it contributed, or why nothing.
    pub corpus_lines: Vec<String>,
    /// One warning per slop flag that matched no example.
    pub warnings: Vec<String>,
    /// The text of `stats.md`.
    pub report: String,
}

/// Reads every input, filters the examples, and writes `train.jsonl`,
/// `slop.jsonl`, `preferences.jsonl` and `stats.md` to `output_dir`.
///
/// # Errors
///
/// `DataError::Io` when an input can't be read or an output can't be written,
/// and `DataError::Json` when an input does not parse.
pub fn build_training_set(inputs: &Inputs, output_dir: &Path) -> Result<Built, DataError> {
    let mut skip_reasons = Vec::new();
    let (examples, corpus_lines) = read_all_examples(inputs, &mut skip_reasons)?;
    let unique_examples = remove_short_and_duplicate(examples, &mut skip_reasons);
    let slop_flags_by_id = slop_flags::read(&inputs.slop_flags)?;
    let (training_set, flagged_examples, unmatched_flags) =
        slop_flags::separate_flagged(unique_examples, &slop_flags_by_id, &mut skip_reasons);
    let warnings = unmatched_flags
        .iter()
        .map(|unmatched| {
            format!(
                "warning: flag {} matched no example and was NOT applied: {}",
                unmatched.id, unmatched.note
            )
        })
        .collect();
    let preference_file = inputs.datastore.join(CLEVER_VS_READABLE_DPO);
    let preference_pairs = clever_vs_readable::preference_pairs(&read_file(&preference_file)?)?;
    let slop_span_counts = slop_flags::span_counts(&flagged_examples);
    let report = report::render(&ReportInput {
        training_set: &training_set,
        skip_reasons: &skip_reasons,
        preference_pair_count: preference_pairs.len(),
        slop_span_counts: &slop_span_counts,
        unmatched_flags: &unmatched_flags,
    });
    let training_records: Vec<TrainingRecord> =
        training_set.iter().map(TrainingRecord::new).collect();
    fs::create_dir_all(output_dir).map_err(DataError::io(output_dir))?;
    write_jsonl(&output_dir.join("train.jsonl"), &training_records)?;
    write_jsonl(&output_dir.join("slop.jsonl"), &flagged_examples)?;
    write_jsonl(&output_dir.join("preferences.jsonl"), &preference_pairs)?;
    write_file(&output_dir.join("stats.md"), &report)?;
    Ok(Built {
        corpus_lines,
        warnings,
        report,
    })
}

/// Reads every source, curated sets first, so the curated copy of a duplicate
/// is the one kept. Also returns one line per corpus source.
fn read_all_examples(
    inputs: &Inputs,
    skip_reasons: &mut Vec<SkipReason>,
) -> Result<(Vec<Example>, Vec<String>), DataError> {
    let datastore = inputs.datastore.as_path();
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
        &inputs.book_repository,
        skip_reasons,
    )?);
    let fetched = corpus::examples(&inputs.corpus, &inputs.manifest)?;
    let corpus_lines = fetched.reports.iter().map(corpus_line).collect();
    examples.extend(fetched.examples);
    Ok((examples, corpus_lines))
}

/// One line saying what a corpus source contributed, or why nothing.
fn corpus_line(report: &corpus::SourceReport) -> String {
    let name = &report.name;
    match &report.outcome {
        corpus::SourceOutcome::Used {
            examples,
            before_cap,
            licence,
        } if examples < before_cap => {
            format!(
                "  {name:<22} {examples:>6} examples  {}  (capped from {before_cap})",
                licence.name()
            )
        }
        corpus::SourceOutcome::Used {
            examples, licence, ..
        } => {
            format!("  {name:<22} {examples:>6} examples  {}", licence.name())
        }
        corpus::SourceOutcome::LicenceRefused(licence) => {
            format!("  {name:<22} refused, licence {}", licence.name())
        }
        corpus::SourceOutcome::NoMarkdown => format!("  {name:<22} no markdown found"),
        corpus::SourceOutcome::CodeOnly => {
            format!("  {name:<22} code repository, read by the teacher")
        }
    }
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
    let mut book_files = Vec::new();
    collect_book_files(root, &mut book_files)?;
    book_files.sort();
    let mut examples = Vec::new();
    for (path, kind) in book_files {
        let origin = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        match kind {
            BookFile::Chapter => {
                examples.extend(book::examples(&corpus::clean(&read_file(&path)?), &origin))
            }
            BookFile::Code => match code::example(&read_file(&path)?, &path, &origin) {
                Some(example) => examples.push(example),
                None => skip_reasons.push(SkipReason::NoHeaderComment),
            },
            BookFile::Notes => skip_reasons.push(SkipReason::NotesFile),
        }
    }
    Ok(examples)
}

/// Walks `directory` and collects every chapter, notes file and source file with its kind.
fn collect_book_files(
    directory: &Path,
    book_files: &mut Vec<(PathBuf, BookFile)>,
) -> Result<(), DataError> {
    for entry in fs::read_dir(directory).map_err(DataError::io(directory))? {
        let path = entry.map_err(DataError::io(directory))?.path();
        match classify(&path) {
            CorpusFile::Directory => collect_book_files(&path, book_files)?,
            CorpusFile::File(kind) => book_files.push((path, kind)),
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
        (false, false, true) if NOTES_FILES.contains(&name) => CorpusFile::File(BookFile::Notes),
        (false, false, true) => CorpusFile::File(BookFile::Chapter),
        (false, false, false) if code::is_source_file(path) => CorpusFile::File(BookFile::Code),
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
    (!is_first_copy).then_some(SkipReason::Duplicate)
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

    /// A small tree of every input, removed when dropped.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Result<Self, DataError> {
            let root = std::env::temp_dir()
                .join(format!("thor-hammer-build-{name}-{}", std::process::id()));
            let fixture = Self { root };
            let long =
                "A page is a fixed-size block of virtual memory that the kernel maps. ".repeat(4);
            fixture.write(
                "store/readability_training.md",
                &format!("# Set\n---\n### Instruction\nWhat is a page?\n### Response\n{long}\n"),
            )?;
            let sft = format!(
                r#"{{"messages": [{{"role": "user", "content": "Rewrite this loop."}}, {{"role": "assistant", "content": "{long}"}}], "meta": {{"id": "rs-x"}}}}"#
            );
            fixture.write(
                "store/clever_vs_readable/clever_vs_readable_sft.jsonl",
                &sft,
            )?;
            let dpo = r#"{"prompt":[{"role":"user","content":"Q"}],"chosen":[{"role":"assistant","content":"good"}],"rejected":[{"role":"assistant","content":"bad"}],"meta":{"id":"rs-x"}}"#;
            fixture.write("store/clever_vs_readable/clever_vs_readable_dpo.jsonl", dpo)?;
            let export = format!(
                r#"[{{"uuid": "c1", "chat_messages": [{{"sender": "human", "content": [{{"type": "text", "text": "What is a futex?"}}]}}, {{"sender": "assistant", "content": [{{"type": "text", "text": "A fast userspace mutex. {long}"}}]}}]}}]"#
            );
            fixture.write("store/work/extracted/conversations.json", &export)?;
            fixture.write(
                "store/chat_0.md",
                &format!(
                    "#Question\nWhat is a TLB?\n#Answer\nA cache of page translations. {long}\n"
                ),
            )?;
            fixture.write(
                "books/book/ch01.md",
                &format!("# Memory\n\n## Pages\n\n{long}\n"),
            )?;
            fixture.write("books/book/WORKLOG.md", "notes")?;
            fixture.write(
                "books/lab/topk.rs",
                &format!("//! Top K frequent elements, using a heap. {long}\nfn main() {{}}\n"),
            )?;
            fixture.write("books/lab/bare.rs", "fn main() {}\n")?;
            fixture.write("books/target/skip.md", "build output")?;
            fixture.write("books/.git/HEAD.md", "hidden")?;
            fixture.write(
                "corpus/eng-practices/review/index.md",
                &format!("# Review\n\n## Design\n\n{long}\n"),
            )?;
            fixture.write("corpus/closed/review/a.md", "# Closed\n")?;
            fixture.write("corpus/empty/README.txt", "nothing")?;
            let manifest = "source\tkind\tcommit\tlicence_file\tlicence\neng-practices\tdocs\tabc\tLICENSE\tAttribution 4.0 International\nclosed\tdocs\tabc\tLICENSE\tAll rights reserved\nempty\tdocs\tabc\tLICENSE\tMIT License\nlib\tcode\tabc\tLICENSE\tMIT License\n";
            fixture.write("manifest.tsv", manifest)?;
            Ok(fixture)
        }

        fn write(&self, path: &str, text: &str) -> Result<(), DataError> {
            let file = self.root.join(path);
            let parent = file.parent().unwrap_or(&self.root);
            fs::create_dir_all(parent).map_err(DataError::io(parent))?;
            fs::write(&file, text).map_err(DataError::io(&file))
        }

        fn inputs(&self) -> Inputs {
            Inputs {
                datastore: self.root.join("store"),
                book_repository: self.root.join("books"),
                corpus: self.root.join("corpus"),
                manifest: self.root.join("manifest.tsv"),
                slop_flags: self.root.join("flags.jsonl"),
            }
        }

        fn read(&self, path: &str) -> Result<String, DataError> {
            read_file(&self.root.join(path))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn builds_every_output_from_every_input() -> Result<(), DataError> {
        let fixture = Fixture::new("all")?;
        let built = build_training_set(&fixture.inputs(), &fixture.root.join("out"))?;
        let train = fixture.read("out/train.jsonl")?;
        let sources: Vec<String> = train
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|record| {
                record
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .map(String::from)
            })
            .collect();
        assert_eq!(
            sources,
            [
                "readability",
                "clever_vs_readable",
                "chat",
                "chat",
                "book",
                "code",
                "corpus"
            ]
        );
        assert_eq!(fixture.read("out/preferences.jsonl")?.lines().count(), 1);
        assert_eq!(fixture.read("out/slop.jsonl")?, "");
        assert!(built.report.contains("1 markdown files of notes"));
        assert!(
            built
                .report
                .contains("1 code files without a header comment")
        );
        assert_eq!(
            built.corpus_lines,
            [
                "  eng-practices               1 examples  CC-BY",
                "  closed                 refused, licence unrecognised",
                "  empty                  no markdown found",
                "  lib                    code repository, read by the teacher",
            ]
        );
        assert!(built.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn a_flag_moves_its_example_to_slop_and_a_stale_flag_warns() -> Result<(), DataError> {
        let fixture = Fixture::new("flags")?;
        let first = build_training_set(&fixture.inputs(), &fixture.root.join("out"))?;
        let train = fixture.read("out/train.jsonl")?;
        let id = train
            .lines()
            .next()
            .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .and_then(|record| {
                record
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(String::from)
            })
            .unwrap_or_default();
        fixture.write("flags.jsonl", &format!("{{\"id\":\"{id}\",\"note\":\"opens with filler\",\"spans\":[]}}\n{{\"id\":\"gone\",\"note\":\"old\",\"spans\":[]}}\n{{\"id\":\"also-gone\",\"note\":\"older\",\"spans\":[]}}\n"))?;
        let second = build_training_set(&fixture.inputs(), &fixture.root.join("out"))?;
        assert_eq!(
            fixture.read("out/train.jsonl")?.lines().count(),
            train.lines().count() - 1
        );
        assert_eq!(fixture.read("out/slop.jsonl")?.lines().count(), 1);
        assert_eq!(
            second.warnings,
            [
                "warning: flag also-gone matched no example and was NOT applied: older",
                "warning: flag gone matched no example and was NOT applied: old"
            ]
        );
        assert_ne!(first.report, second.report);
        Ok(())
    }

    #[test]
    fn a_missing_input_is_an_error_naming_the_file() -> Result<(), DataError> {
        let fixture = Fixture::new("missing")?;
        fs::remove_file(fixture.root.join("store/chat_0.md"))
            .map_err(DataError::io(&fixture.root))?;
        let error = build_training_set(&fixture.inputs(), &fixture.root.join("out"))
            .err()
            .map(|error| error.to_string());
        assert!(error.is_some_and(|message| message.contains("chat_0.md")));
        Ok(())
    }

    #[test]
    fn a_capped_source_says_so() {
        let report = corpus::SourceReport {
            name: "kubernetes-website".to_string(),
            outcome: corpus::SourceOutcome::Used {
                examples: 10,
                before_cap: 40,
                licence: corpus::Licence::CreativeCommons,
            },
        };
        assert_eq!(
            corpus_line(&report),
            "  kubernetes-website         10 examples  CC-BY  (capped from 40)"
        );
    }

    #[test]
    fn the_usual_inputs_sit_under_home() {
        let inputs = Inputs::under_home(Path::new("/home/a"));
        assert_eq!(inputs.datastore, Path::new("/home/a").join(DATASTORE));
        assert_eq!(inputs.slop_flags, PathBuf::from(SLOP_FLAGS_FILE));
    }

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
            CorpusFile::File(BookFile::Notes)
        ));
        assert!(matches!(
            classify(Path::new("book/ch01.md")),
            CorpusFile::File(BookFile::Chapter)
        ));
        assert!(matches!(
            classify(Path::new("lab/topk.rs")),
            CorpusFile::File(BookFile::Code)
        ));
        assert!(matches!(
            classify(Path::new("lab/notes.txt")),
            CorpusFile::Ignored
        ));
    }
}
