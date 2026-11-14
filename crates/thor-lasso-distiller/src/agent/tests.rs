//! Tests of `lasso agent`, with a fake `gh` that answers from recorded
//! output and a fake openbatrangs, both in `tests/fixtures/agent`.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    arguments::Arguments,
    export, jsonl, main_with,
    process::{self, Finish},
    run::{RunRecord, Scratch},
    score::Score,
    task::{self, Task},
    tasks::GitHub,
};
use crate::error::DistillError;

type Outcome = Result<(), DistillError>;

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/agent")
        .join(name)
}

fn folder() -> Result<TempDir, DistillError> {
    tempfile::tempdir().map_err(DistillError::io("temporary folder"))
}

fn text(path: &Path) -> Result<String, DistillError> {
    fs::read_to_string(path).map_err(DistillError::io(path))
}

/// The text of the error in `result`, or nothing when it is not one.
fn failure<T>(result: Result<T, DistillError>) -> String {
    result
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default()
}

fn agent(list: &[&str]) -> Result<String, DistillError> {
    agent_owned(&words(list))
}

/// Writes an executable shell script named `name` into `folder`.
fn script(folder: &Path, name: &str, body: &str) -> Result<PathBuf, DistillError> {
    let path = folder.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).map_err(DistillError::io(&path))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
        .map_err(DistillError::io(&path))?;

    Ok(path)
}

fn git(folder: &Path, arguments: &[&str]) -> Result<String, DistillError> {
    let stdout = process::output(
        Command::new("git")
            .arg("-C")
            .arg(folder)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(arguments),
    )?;

    Ok(String::from_utf8_lossy(&stdout).trim().to_string())
}

/// A repository at `root/me/demo` with a base commit and a merge commit that
/// adds a test file and the fix; returns the two hashes.
fn demo_repository(root: &Path) -> Result<(String, String), DistillError> {
    let repository = root.join("me/demo");
    let tests = repository.join("tests");
    fs::create_dir_all(&tests).map_err(DistillError::io(&tests))?;
    git(&repository, &["init", "--quiet"])?;
    fs::write(repository.join("README.md"), "demo\n").map_err(DistillError::io(&repository))?;
    git(&repository, &["add", "--all"])?;
    git(&repository, &["commit", "--quiet", "--message", "base"])?;
    let base = git(&repository, &["rev-parse", "HEAD"])?;

    fs::write(tests.join("check.rs"), "// test\n").map_err(DistillError::io(&tests))?;
    fs::write(repository.join("fix.rs"), "// fix\n").map_err(DistillError::io(&repository))?;
    git(&repository, &["add", "--all"])?;
    git(&repository, &["commit", "--quiet", "--message", "fix"])?;
    let merge = git(&repository, &["rev-parse", "HEAD"])?;

    Ok((base, merge))
}

fn demo_task(issue: u64, prompt: &str, split: &str, commits: &(String, String)) -> Task {
    Task {
        id: format!("me/demo#{issue}"),
        repo: "me/demo".to_string(),
        issue,
        pr: issue + 1,
        prompt: prompt.to_string(),
        base: commits.0.clone(),
        merge: commits.1.clone(),
        test_files: vec!["tests/check.rs".to_string()],
        check: "test -f done.txt && grep -q test tests/check.rs && test ! -f fix.rs \
                && test \"$(git rev-list --count HEAD)\" = 1"
            .to_string(),
        split: split.to_string(),
    }
}

fn record(id: &str, passed: bool, outcome: &str, steps: u64, transcript: Value) -> RunRecord {
    RunRecord {
        run_id: format!("{id}@1"),
        task_id: id.to_string(),
        repo: "me/demo".to_string(),
        split: "train".to_string(),
        prompt: "p".to_string(),
        passed,
        steps,
        outcome: outcome.to_string(),
        wall_seconds: 1.0,
        transcript,
    }
}

#[test]
fn arguments_are_read_and_checked() -> Outcome {
    let parsed = Arguments::parse(&words(&["runs.jsonl", "--out", "o", "--", "--thor", "-v"]))?;

    assert_eq!(parsed.positional, ["runs.jsonl"]);
    assert_eq!(parsed.required("out")?, "o");
    assert_eq!(parsed.passed_on, ["--thor", "-v"]);
    assert_eq!(parsed.or("labels", "none"), "none");
    assert_eq!(parsed.number("timeout", 7)?, 7);
    assert_eq!(parsed.single("runs file")?, "runs.jsonl");
    assert!(parsed.allow_only(&["out"]).is_ok());

    let two = Arguments::parse(&words(&["a", "b", "--timeout", "soon"]))?;

    assert_eq!(
        failure(Arguments::parse(&words(&["--out"]))),
        "--out needs a value"
    );
    assert_eq!(failure(two.required("out")), "--out is required");
    assert_eq!(
        failure(two.number("timeout", 1)),
        "--timeout needs a number, not soon"
    );
    assert!(failure(two.single("runs file")).starts_with("needs one runs file"));
    assert!(failure(two.allow_only(&[])).starts_with("unknown option --timeout"));

    Ok(())
}

#[test]
fn an_unknown_or_missing_subcommand_shows_the_usage() {
    assert!(failure(agent(&[])).starts_with("usage: lasso agent"));
    assert!(failure(agent(&["fly"])).starts_with("usage: lasso agent"));
}

#[test]
fn test_files_are_told_apart_from_code() {
    let tests = [
        "tests/cli.rs",
        "src/chat/tests.rs",
        "src/testing.rs",
        "src/parse_test.rs",
        "web/tests/a.mjs",
    ];
    let code = ["src/cli.rs", "src/contest.rs", "README.md"];

    assert!(tests.iter().all(|path| task::is_test_file(path)));
    assert!(!code.iter().any(|path| task::is_test_file(path)));

    assert_eq!(task::check_for("tests/cli.rs"), Some("cargo test"));
    assert_eq!(task::check_for("web/tests/a.mjs"), None);
    assert_eq!(task::check_for("tests/Makefile"), None);
}

#[test]
fn the_split_is_stable() {
    assert_eq!(task::fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(
        task::split_of("arpanpathak/openbatrangs#38"),
        task::split_of("arpanpathak/openbatrangs#38")
    );

    let ids: Vec<String> = (1..=1000).map(|issue| format!("me/demo#{issue}")).collect();
    let eval = ids
        .iter()
        .filter(|id| task::split_of(id) == task::EVAL)
        .count();

    assert!((150..250).contains(&eval), "{eval} of 1000 in eval");
}

/// Runs `lasso agent tasks` against the recorded `gh` output and returns
/// what it printed and the tasks it wrote.
fn recorded_tasks() -> Result<(String, Vec<Task>), DistillError> {
    let folder = folder()?;
    let out = folder.path().join("tasks.jsonl");
    let gh = fixture("gh").display().to_string();
    let path = out.display().to_string();
    let repo = "arpanpathak/openbatrangs";
    let printed = agent(&["tasks", "--repo", repo, "--gh", &gh, "--out", &path])?;

    Ok((printed, jsonl::read(&out)?))
}

#[test]
fn tasks_come_from_pull_requests_that_close_issues_and_change_tests() -> Outcome {
    let (printed, tasks) = recorded_tasks()?;
    let ids: Vec<&str> = tasks.iter().map(|task| task.id.as_str()).collect();

    assert!(printed.starts_with("2 tasks ("), "{printed}");
    assert!(printed.contains("from 4 merged pull requests"), "{printed}");
    assert_eq!(
        ids,
        ["arpanpathak/openbatrangs#38", "arpanpathak/openbatrangs#24"]
    );

    Ok(())
}

#[test]
fn a_task_starts_at_the_first_parent_with_the_issue_as_prompt() -> Outcome {
    let (_, tasks) = recorded_tasks()?;
    let update = &tasks[0];

    assert_eq!((update.issue, update.pr), (38, 39));
    assert_eq!(update.base, "3938b9c52154fb8384d29b0a08bd4c362be5855c");
    assert_eq!(update.merge, "454c2b891bea1d225bd70c1066782ae6013d1872");
    assert_eq!(update.test_files, ["tests/cli.rs"]);
    assert_eq!(update.check, "cargo test");
    assert_eq!(update.split, task::split_of(&update.id));

    let title = "Check for new releases and offer to install them\n\n";

    assert!(update.prompt.starts_with(title));

    Ok(())
}

#[test]
fn a_task_that_closes_several_issues_asks_for_all_of_them() -> Outcome {
    let (_, tasks) = recorded_tasks()?;
    let prompt = &tasks[1].prompt;

    assert!(prompt.contains("Fold agent steps in the TUI transcript"));
    assert!(prompt.contains("Let the model ask the user a question"));

    Ok(())
}
#[test]
fn a_failing_or_odd_gh_is_an_error() -> Outcome {
    let folder = folder()?;
    let github = |program: PathBuf| GitHub {
        program,
        repo: "me/demo".to_string(),
    };
    let orphan = github(script(
        folder.path(),
        "orphan",
        r#"echo '{"parents": []}'"#,
    )?);
    let garbled = github(script(folder.path(), "garbled", "echo not json")?);
    let failing = github(script(
        folder.path(),
        "failing",
        "echo 'not logged in' >&2; exit 1",
    )?);

    assert_eq!(
        failure(orphan.first_parent("abc")),
        "commit abc has no parent"
    );
    assert!(matches!(
        garbled.merged_pulls(),
        Err(DistillError::Json { .. })
    ));
    assert!(failure(failing.issues()).ends_with("failed: not logged in"));
    assert!(matches!(
        github(folder.path().join("missing")).issues(),
        Err(DistillError::Io { .. })
    ));

    Ok(())
}

/// The arguments of `lasso agent run` on five demo tasks, four of them in
/// train, with a one-second limit; the folder holds the repository.
fn demo_run(folder: &Path) -> Result<Vec<String>, DistillError> {
    let commits = demo_repository(folder)?;
    let tasks_path = folder.join("tasks.jsonl");
    let tasks = [
        demo_task(1, "fix it", "train", &commits),
        demo_task(2, "give up", "train", &commits),
        demo_task(3, "crash", "train", &commits),
        demo_task(4, "hang", "train", &commits),
        demo_task(5, "held out", "eval", &commits),
    ];
    jsonl::write(&tasks_path, &tasks)?;

    let options = [
        ("--tasks", tasks_path),
        ("--openbatrangs", fixture("openbatrangs")),
        ("--out", folder.join("runs.jsonl")),
        ("--clone-url", folder.join("{repo}")),
    ];
    let mut arguments = words(&["run", "--split", "train", "--timeout", "1"]);
    arguments.extend(
        options
            .into_iter()
            .flat_map(|(name, path)| [name.to_string(), path.display().to_string()]),
    );
    arguments.extend(words(&["--", "--thor"]));

    Ok(arguments)
}

fn agent_owned(arguments: &[String]) -> Result<String, DistillError> {
    let mut out = Vec::new();
    main_with(arguments, &mut out)?;

    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn how_it_ended(record: &RunRecord) -> String {
    format!(
        "{} {} {} {}",
        record.task_id, record.passed, record.outcome, record.steps
    )
}

#[test]
fn runs_record_each_way_a_task_can_end() -> Outcome {
    let folder = folder()?;
    let printed = agent_owned(&demo_run(folder.path())?)?;
    let records: Vec<RunRecord> = jsonl::read(&folder.path().join("runs.jsonl"))?;
    let ends: Vec<String> = records.iter().map(how_it_ended).collect();

    assert!(
        printed.starts_with("4 train tasks, 0 already in"),
        "{printed}"
    );
    assert!(
        printed.contains("me/demo#1: passed, 2 steps, answered"),
        "{printed}"
    );
    assert_eq!(
        ends,
        [
            "me/demo#1 true answered 2",
            "me/demo#2 false step_limit 2",
            "me/demo#3 false error 0",
            "me/demo#4 false timeout 0",
        ]
    );
    assert_eq!(records[0].transcript["model"], "fake");
    assert_eq!(records[2].transcript, Value::Null);

    Ok(())
}

#[test]
fn a_second_run_skips_the_tasks_already_recorded() -> Outcome {
    let folder = folder()?;
    let arguments = demo_run(folder.path())?;
    agent_owned(&arguments)?;

    let again = agent_owned(&arguments)?;
    let records: Vec<RunRecord> = jsonl::read(&folder.path().join("runs.jsonl"))?;

    assert!(again.starts_with("4 train tasks, 4 already in"), "{again}");
    assert!(again.ends_with(", 0 left\n"), "{again}");
    assert_eq!(records.len(), 4);

    Ok(())
}
#[test]
fn a_run_refuses_bad_options_and_missing_repositories() -> Outcome {
    let folder = folder()?;
    let commits = (String::from("a"), String::from("b"));
    let tasks_path = folder.path().join("tasks.jsonl");
    jsonl::write(&tasks_path, &[demo_task(1, "fix it", "train", &commits)])?;

    let tasks_arg = tasks_path.display().to_string();
    let runs = folder.path().join("runs.jsonl").display().to_string();
    let missing = format!("{}/nowhere/{{repo}}", folder.path().display());
    let base = [
        "run",
        "--tasks",
        &tasks_arg,
        "--openbatrangs",
        "true",
        "--out",
        &runs,
    ];
    let with = |extra: &[&str]| agent(&[&base[..], extra].concat());

    assert_eq!(
        failure(with(&["--split", "test"])),
        "--split is train or eval, not test"
    );
    assert!(matches!(
        with(&["--split", "eval", "--colour", "red"]),
        Err(DistillError::Usage(_))
    ));
    assert!(matches!(
        with(&["--split", "train", "--clone-url", &missing]),
        Err(DistillError::Command(_))
    ));

    Ok(())
}

#[test]
fn a_scratch_folder_that_is_already_gone_is_reported() -> Outcome {
    let scratch = Scratch::new()?;
    fs::remove_dir_all(&scratch.path).map_err(DistillError::io(&scratch.path))?;
    drop(scratch);

    Ok(())
}

#[test]
fn programs_are_run_and_stopped() -> Outcome {
    let missing = Path::new("/nonexistent/program");

    assert!(matches!(
        process::output(&mut Command::new(missing)),
        Err(DistillError::Io { .. })
    ));
    assert!(matches!(
        process::finish_within(&mut Command::new(missing), Duration::from_secs(1)),
        Err(DistillError::Io { .. })
    ));
    assert_eq!(
        process::finish_within(
            Command::new("sh").args(["-c", "exit 3"]),
            Duration::from_secs(5)
        )?,
        Finish::Exited(false)
    );

    Ok(())
}

#[test]
fn a_score_adds_up_the_runs() -> Outcome {
    let records = [
        record("a#1", true, "answered", 4, Value::Null),
        record("a#2", false, "step_limit", 10, Value::Null),
        record("a#3", false, "error", 0, Value::Null),
        record("a#4", false, "timeout", 0, Value::Null),
    ];
    let score = Score::of(&records);

    assert_eq!(
        score.table(),
        "runs        4\npassed      1 (25.0%)\nmean steps  3.5\nstep limit  1\nerrors      1\ntimeouts    1\n"
    );
    assert_eq!(Score::of(&[]).mean_steps, 0.0);

    let folder = folder()?;
    let path = folder.path().join("runs.jsonl");
    jsonl::write(&path, &records)?;

    let printed = agent(&["score", &path.display().to_string()])?;

    assert!(printed.starts_with("runs        4\n"));
    assert!(matches!(agent(&["score"]), Err(DistillError::Usage(_))));

    Ok(())
}

/// A runs file with two passed, answered runs (the second flagged as slop
/// in a labels file), one at the step limit and one failed; returns the
/// paths of the runs, the labels and the output, as arguments.
fn export_files(folder: &Path) -> Result<[String; 3], DistillError> {
    let transcript = json!({
        "tools": [{"type": "function", "function": {"name": "read"}}],
        "messages": [
            {"role": "user", "content": "Fix it."},
            {"role": "assistant", "content": "Let me check.", "tool_calls": [{"id": "1"}]},
            {"role": "tool", "tool_call_id": "1", "content": "text"},
            {"role": "assistant", "content": "Fixed.", "tool_calls": []},
        ],
    });
    let records = [
        record("a#1", true, "answered", 2, transcript.clone()),
        record("a#2", true, "answered", 2, transcript),
        record("a#3", true, "step_limit", 9, Value::Null),
        record("a#4", false, "answered", 2, Value::Null),
    ];
    let runs = folder.join("runs.jsonl");
    let labels = folder.join("flags.jsonl");
    let flag = "{\"id\": \"a#2@1\", \"note\": \"narrates\", \"spans\": []}\n";
    jsonl::write(&runs, &records)?;
    fs::write(&labels, flag).map_err(DistillError::io(&labels))?;

    Ok([runs, labels, folder.join("train.jsonl")].map(|path| path.display().to_string()))
}

#[test]
fn export_keeps_passed_answered_runs_without_narration_or_slop() -> Outcome {
    let folder = folder()?;
    let [runs, labels, out] = export_files(folder.path())?;
    let printed = agent(&["export", &runs, "--out", &out, "--labels", &labels])?;
    let lines: Vec<Value> = jsonl::read(Path::new(&out))?;
    let expected = format!("2 of 4 runs passed, 1 marked as slop, 1 lines written to {out}\n");

    assert_eq!(printed, expected);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["tools"][0]["function"]["name"], "read");
    assert_eq!(lines[0]["messages"][1]["content"], "");
    assert_eq!(lines[0]["messages"][1]["tool_calls"][0]["id"], "1");
    assert_eq!(lines[0]["messages"][3]["content"], "Fixed.");

    Ok(())
}

#[test]
fn export_without_labels_keeps_every_passed_run() -> Outcome {
    let folder = folder()?;
    let [runs, _, out] = export_files(folder.path())?;
    let missing = folder.path().join("none.jsonl").display().to_string();
    agent(&["export", &runs, "--out", &out])?;

    assert_eq!(text(Path::new(&out))?.lines().count(), 2);
    assert_eq!(
        export::training_line(&Value::Null),
        json!({"tools": [], "messages": []})
    );
    assert!(matches!(
        agent(&["export", &runs, "--out", &out, "--labels", &missing]),
        Err(DistillError::Io { .. })
    ));

    Ok(())
}
#[test]
fn jsonl_files_report_where_they_fail() -> Outcome {
    let folder = folder()?;
    let path = folder.path().join("bad.jsonl");
    fs::write(&path, "{\"id\": \"a\"}\n\nnot json\n").map_err(DistillError::io(&path))?;

    assert!(matches!(
        jsonl::read::<Value>(&path),
        Err(DistillError::Json { line: 3, .. })
    ));
    assert!(jsonl::read_if_present::<Value>(&folder.path().join("none"))?.is_empty());
    assert!(matches!(
        jsonl::read_if_present::<Value>(folder.path()),
        Err(DistillError::Io { .. })
    ));
    assert!(matches!(
        jsonl::write(folder.path(), &[1]),
        Err(DistillError::Io { .. })
    ));
    assert!(matches!(
        jsonl::append(folder.path(), &1),
        Err(DistillError::Io { .. })
    ));

    Ok(())
}

#[test]
fn the_lasso_command_line_reaches_agent() -> Outcome {
    let folder = folder()?;
    let path = folder.path().join("runs.jsonl");
    fs::write(&path, "").map_err(DistillError::io(&path))?;

    let mut out = Vec::new();
    crate::cli::main_with(
        &words(&["agent", "score", &path.display().to_string()]),
        None,
        &mut out,
    )?;

    assert!(String::from_utf8_lossy(&out).starts_with("runs        0\npassed      0 (0.0%)"));
    assert!(crate::cli::usage().contains("lasso agent"));

    Ok(())
}
