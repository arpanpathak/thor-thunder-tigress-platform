use super::*;

struct Workspace {
    root: PathBuf,
    paths: Paths,
}

impl Workspace {
    fn new(name: &str) -> Result<Self, DataError> {
        let root =
            std::env::temp_dir().join(format!("thor-hammer-teacher-{name}-{}", std::process::id()));
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

        let heap = "def push(heap, item):\n    heap.append(item)\n".repeat(60);
        workspace.write("corpus/lib/src/heap.py", &heap)?;

        Ok(workspace)
    }

    fn write(&self, path: &str, text: &str) -> Result<(), DataError> {
        let file = self.root.join(path);
        let parent = file.parent().unwrap_or(&self.root);
        fs::create_dir_all(parent).map_err(DataError::io(parent))?;

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

    assert_eq!(
        words(&[]),
        Command::Check {
            source: DEFAULT_SOURCE.into(),
            output: DEFAULT_OUTPUT.into()
        }
    );
    assert_eq!(
        words(&["check", "a", "b"]),
        Command::Check {
            source: "a".into(),
            output: "b".into()
        }
    );
    assert_eq!(
        words(&["pick", "3", "trpl"]),
        Command::Pick {
            count: 3,
            sources: vec!["trpl".to_string()]
        }
    );
    assert_eq!(words(&["pick", "many"]), Command::Usage);
}

#[test]
fn writes_passing_entries_and_reports_failing_ones() -> Result<(), DataError> {
    let workspace = Workspace::new("check")?;
    workspace.write("teacher/a.md", &format!("{GOOD}---\n<!-- source: notes -->\n### User\nQ\n\n### Assistant\nGreat question!\n\n```python\nassert 1 + 1 == 2\n```\n---\n<!-- source: notes -->\n### User\nQ\n\n### Assistant\n```python\nassert sorted([2, 1]) == [1, 2]\n```\n"))?;

    let unknown = GOOD.replace("section: s1", "section: nowhere");
    let more = format!("{unknown}---\n### User\nno source comment\n");
    workspace.write("teacher/more/b.md", &more)?;

    let result = check(&workspace.paths)?;

    assert!(!result.all_passed);
    assert!(result.summary().contains("| Passed every check | 2 |"));
    assert!(result.report.contains("turn 2: slop: Great question"));
    assert!(result.report.contains("| python | 1 |"));
    assert!(result.report.contains("section nowhere is not in"));
    assert!(result.report.contains("format: no <!-- source"));

    let chat = workspace.read("data/teacher.jsonl")?;

    assert!(chat.contains("\"source_text\":\"A long passage about heaps.\""));
    assert!(
        chat.contains("\"source\":\"teacher\",\"origin\":\"book/heaps.md\",\"entry\":\"a.md#1\"")
    );
    assert!(chat.contains("\"licence\":\"MIT\""));

    let preferences = workspace.read("data/teacher_preferences.jsonl")?;

    assert!(
        preferences.contains("\"rejected\":[{\"role\":\"assistant\",\"content\":\"Great question!")
    );
    assert!(result.report.contains("| book | 1 |"));
    assert!(
        result
            .report
            .contains("| Rejected answers spark also catches | 1 of 1 |")
    );

    Ok(())
}

#[test]
fn a_set_without_grounded_entries_needs_no_sections() -> Result<(), DataError> {
    let workspace = Workspace::new("plain")?;
    let note = "<!-- source: notes -->\n### User\nQ\n\n### Assistant\nA plain answer.\n";
    workspace.write("teacher/a.md", note)?;
    fs::remove_file(workspace.root.join("data/train.jsonl"))
        .map_err(DataError::io(&workspace.root))?;

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

    let note = "<!-- source: notes -->\n### User\nQ\n\n### Assistant\nA.\n";
    workspace.write("teacher/a.md", note)?;

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

    assert_eq!(
        (paths.corpus, paths.manifest),
        (PathBuf::from(CORPUS), PathBuf::from(MANIFEST))
    );
}
