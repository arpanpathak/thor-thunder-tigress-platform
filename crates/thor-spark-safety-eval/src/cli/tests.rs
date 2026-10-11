use super::*;

/// A fresh folder for one test, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Outcome<Folder> {
        let path = std::env::temp_dir().join(format!("spark-cli-{}-{name}", std::process::id()));
        fs::create_dir_all(&path).map_err(EvalError::io(&path))?;

        Ok(Folder(path))
    }

    fn file(&self, name: &str, text: &str) -> Outcome<PathBuf> {
        let path = self.0.join(name);
        let parent = path.parent().unwrap_or(&self.0);
        fs::create_dir_all(parent).map_err(EvalError::io(parent))?;
        fs::write(&path, text).map_err(EvalError::io(&path))?;

        Ok(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(ToString::to_string).collect()
}

#[test]
fn reads_each_command() -> Outcome {
    assert_eq!(
        Command::from_args(&args(&["rs", "src"]))?,
        Command::Rust(vec![PathBuf::from("src")])
    );
    assert_eq!(
        Command::from_args(&args(&["text", "a.md"]))?,
        Command::Text(vec![PathBuf::from("a.md")])
    );

    let expected = ScoreOptions {
        input: PathBuf::from("runs/base.jsonl"),
        field: "text".to_string(),
        code_field: Some("code".to_string()),
        label: "base".to_string(),
        output: PathBuf::from("runs/base.scored.jsonl"),
    };

    assert_eq!(
        Command::from_args(&args(&["score", "runs/base.jsonl", "--code-field", "code"]))?,
        Command::Score(expected)
    );

    Ok(())
}

#[test]
fn refuses_what_it_does_not_understand() {
    for words in [
        &[][..],
        &["lint"],
        &["rs"],
        &["score"],
        &["score", "a", "--colour"],
        &["score", "a", "--label"],
    ] {
        assert!(
            matches!(Command::from_args(&args(words)), Err(EvalError::Usage(_))),
            "{words:?}"
        );
    }
}

#[test]
fn checks_rust_files_and_skips_build_output() -> Outcome {
    let folder = Folder::new("rs")?;
    let good = "/// Adds.\npub fn add(a: i32, b: i32) -> i32 { a + b }\n";
    folder.file("src/good.rs", good)?;
    folder.file("src/bad.rs", "fn a() { b().unwrap(); }\n")?;
    folder.file("src/broken.rs", "fn a( {\n")?;
    folder.file("target/skip.rs", "fn a() { b().unwrap(); }\n")?;
    folder.file(".hidden/skip.rs", "fn a() { b().unwrap(); }\n")?;

    let report = Command::Rust(vec![folder.0.clone()]).run()?;

    assert_eq!(report.exit, Exit::Problems);
    assert!(
        report
            .lines
            .iter()
            .any(|line| line.ends_with("bad.rs:1: R1 no unwrap: .unwrap()"))
    );
    assert!(
        report
            .lines
            .iter()
            .any(|line| line.contains("broken.rs: does not parse"))
    );
    assert_eq!(
        report.lines.last().map(String::as_str),
        Some("3 files, 2 problems")
    );

    Ok(())
}

#[test]
fn clean_rust_is_clean() -> Outcome {
    let folder = Folder::new("clean")?;
    let file = folder.file("a.rs", "fn add(a: i32) -> i32 { a + 1 }\n")?;

    assert_eq!(Command::Rust(vec![file]).run()?.exit, Exit::Clean);

    Ok(())
}

#[test]
fn finds_slop_in_text_with_its_line() -> Outcome {
    let folder = Folder::new("text")?;
    let file = folder.file("answer.md", "Fine.\nGreat question! Here it is.\n")?;
    let report = Command::Text(vec![file]).run()?;

    assert_eq!(report.exit, Exit::Problems);
    assert!(report.lines[0].ends_with("answer.md:2: flattery and filler openers: Great question"));
    assert!(report.lines[1].ends_with("1 phrases, 0 em dashes, 6 words"));

    Ok(())
}

#[test]
fn scores_a_run_and_writes_each_line_back() -> Outcome {
    let folder = Folder::new("score")?;
    let lines = "{\"text\":\"Great question!\",\"code\":\"fn a() { b().unwrap(); }\"}\n\n{\"text\":\"Plain.\",\"code\":\"\"}\n";
    let run = folder.file("run.jsonl", lines)?;
    let report = Command::from_args(&args(&[
        "score",
        &run.to_string_lossy(),
        "--code-field",
        "code",
        "--label",
        "base",
    ]))?
    .run()?;

    assert!(report.lines[1].starts_with("| base | 2 | 1 |"));

    let written =
        fs::read_to_string(folder.0.join("run.scored.jsonl")).map_err(EvalError::io(&folder.0))?;
    let first: Value = serde_json::from_str(written.lines().next().unwrap_or_default())
        .map_err(EvalError::json(&folder.0, 1))?;

    assert_eq!(first["spark_run"], "base");
    assert_eq!(first["spark"]["rules"]["no_unwrap"], "fail");

    Ok(())
}

#[test]
fn a_line_without_the_field_is_named() -> Outcome {
    let folder = Folder::new("missing")?;
    let run = folder.file("run.jsonl", "{\"text\":\"ok\"}\n{\"other\":1}\n")?;
    let outcome = Command::from_args(&args(&["score", &run.to_string_lossy()]))?.run();

    assert!(outcome.is_err_and(|error| {
        error
            .to_string()
            .ends_with("run.jsonl:2: no string field \"text\"")
    }));

    let bad = folder.file("bad.jsonl", "not json\n")?;

    assert!(matches!(
        Command::from_args(&args(&["score", &bad.to_string_lossy()]))?.run(),
        Err(EvalError::Json { line: 1, .. })
    ));

    Ok(())
}
