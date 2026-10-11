use super::*;
use crate::example::Source;

/// A small tree of every input, removed when dropped.
struct Fixture(tempfile::TempDir);

/// A paragraph long enough to be kept as an example.
fn long() -> String {
    "A page is a fixed-size block of virtual memory that the kernel maps. ".repeat(4)
}

impl Fixture {
    fn new() -> Result<Self, DataError> {
        let fixture =
            Self(tempfile::tempdir().map_err(DataError::io(Path::new("temporary folder")))?);

        fixture.write_datastore()?;
        fixture.write_books()?;
        fixture.write_corpus()?;

        Ok(fixture)
    }

    fn root(&self) -> &Path {
        self.0.path()
    }

    /// The hand-written sets and the chat export.
    fn write_datastore(&self) -> Result<(), DataError> {
        let long = long();
        let readability =
            format!("# Set\n---\n### Instruction\nWhat is a page?\n### Response\n{long}\n");
        let sft = format!(
            r#"{{"messages": [{{"role": "user", "content": "Rewrite this loop."}}, {{"role": "assistant", "content": "{long}"}}], "meta": {{"id": "rs-x"}}}}"#
        );
        let dpo = r#"{"prompt":[{"role":"user","content":"Q"}],"chosen":[{"role":"assistant","content":"good"}],"rejected":[{"role":"assistant","content":"bad"}],"meta":{"id":"rs-x"}}"#;
        let export = format!(
            r#"[{{"uuid": "c1", "chat_messages": [{{"sender": "human", "content": [{{"type": "text", "text": "What is a futex?"}}]}}, {{"sender": "assistant", "content": [{{"type": "text", "text": "A fast userspace mutex. {long}"}}]}}]}}]"#
        );
        let chat =
            format!("#Question\nWhat is a TLB?\n#Answer\nA cache of page translations. {long}\n");

        self.write("store/readability_training.md", &readability)?;
        self.write(
            "store/clever_vs_readable/clever_vs_readable_sft.jsonl",
            &sft,
        )?;
        self.write("store/clever_vs_readable/clever_vs_readable_dpo.jsonl", dpo)?;
        self.write("store/work/extracted/conversations.json", &export)?;
        self.write("store/chat_0.md", &chat)
    }

    /// The book repository: a chapter, code with and without a header, and
    /// files that are skipped.
    fn write_books(&self) -> Result<(), DataError> {
        let long = long();
        let chapter = format!("# Memory\n\n## Pages\n\n{long}\n");
        let topk = format!("//! Top K frequent elements, using a heap. {long}\nfn main() {{}}\n");

        self.write("books/book/ch01.md", &chapter)?;
        self.write("books/book/WORKLOG.md", "notes")?;
        self.write("books/lab/topk.rs", &topk)?;
        self.write("books/lab/bare.rs", "fn main() {}\n")?;
        self.write("books/target/skip.md", "build output")?;
        self.write("books/.git/HEAD.md", "hidden")
    }

    /// The fetched corpus: one usable source, one refused, one empty.
    fn write_corpus(&self) -> Result<(), DataError> {
        let review = format!("# Review\n\n## Design\n\n{}\n", long());
        let manifest = "source\tkind\tcommit\tlicence_file\tlicence\neng-practices\tdocs\tabc\tLICENSE\tAttribution 4.0 International\nclosed\tdocs\tabc\tLICENSE\tAll rights reserved\nempty\tdocs\tabc\tLICENSE\tMIT License\nlib\tcode\tabc\tLICENSE\tMIT License\n";

        self.write("corpus/eng-practices/review/index.md", &review)?;
        self.write("corpus/closed/review/a.md", "# Closed\n")?;
        self.write("corpus/empty/README.txt", "nothing")?;
        self.write("manifest.tsv", manifest)
    }

    fn write(&self, path: &str, text: &str) -> Result<(), DataError> {
        let file = self.root().join(path);
        let parent = file.parent().unwrap_or(self.root());
        fs::create_dir_all(parent).map_err(DataError::io(parent))?;

        fs::write(&file, text).map_err(DataError::io(&file))
    }

    fn inputs(&self) -> Inputs {
        Inputs {
            datastore: self.root().join("store"),
            book_repository: self.root().join("books"),
            corpus: self.root().join("corpus"),
            manifest: self.root().join("manifest.tsv"),
            slop_flags: self.root().join("flags.jsonl"),
        }
    }

    fn read(&self, path: &str) -> Result<String, DataError> {
        read_file(&self.root().join(path))
    }
}

#[test]
fn builds_the_training_set_from_every_input_in_order() -> Result<(), DataError> {
    let fixture = Fixture::new()?;

    build_training_set(&fixture.inputs(), &fixture.root().join("out"))?;

    let train = fixture.read("out/train.jsonl")?;
    let sources: Vec<String> = train
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|record| record["source"].as_str().map(String::from))
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

    Ok(())
}

#[test]
fn reports_what_was_skipped_and_what_each_source_gave() -> Result<(), DataError> {
    let fixture = Fixture::new()?;
    let built = build_training_set(&fixture.inputs(), &fixture.root().join("out"))?;
    let skipped_code = "1 code files without a header comment";

    assert!(built.report.contains("1 markdown files of notes"));
    assert!(built.report.contains(skipped_code));
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
    let fixture = Fixture::new()?;
    let first = build_training_set(&fixture.inputs(), &fixture.root().join("out"))?;
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

    let second = build_training_set(&fixture.inputs(), &fixture.root().join("out"))?;

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
    let fixture = Fixture::new()?;

    fs::remove_file(fixture.root().join("store/chat_0.md"))
        .map_err(DataError::io(fixture.root()))?;

    let error = build_training_set(&fixture.inputs(), &fixture.root().join("out"))
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
