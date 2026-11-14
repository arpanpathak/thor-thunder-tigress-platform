//! `lasso agent run`: one openbatrangs run per task of a split, and the
//! result of the task's check after it.
//!
//! The agent works in a temporary checkout of the task's base commit with the
//! pull request's test files added. The checkout's history is one commit of
//! that state, so later commits, which hold the fix, are not reachable. The
//! test files are written again before the check, so edits to them do not
//! count. A task whose id already has a record in the output file is not run
//! again.

use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    arguments::Arguments,
    jsonl,
    process::{self, Finish},
    task::{self, Task},
};
use crate::error::DistillError;

/// Seconds the agent may take on one task unless `--timeout` says otherwise.
const AGENT_SECONDS: u64 = 3600;

/// Seconds the check may take unless `--check-timeout` says otherwise.
const CHECK_SECONDS: u64 = 1800;

/// Where a repository is cloned from unless `--clone-url` says otherwise;
/// `{repo}` becomes `OWNER/NAME`.
const CLONE_URL: &str = "https://github.com/{repo}.git";

/// The `git` arguments that commit a task's starting state. They set the
/// author and turn off signing, so no `git` configuration is needed.
const START_COMMIT: &[&str] = &[
    "-c",
    "user.name=lasso",
    "-c",
    "user.email=lasso@localhost",
    "-c",
    "commit.gpgsign=false",
    "commit",
    "--quiet",
    "--message",
    "Start of the task",
];

/// The command line of `lasso agent run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOptions {
    /// The tasks file.
    pub tasks: PathBuf,
    /// [`task::TRAIN`] or [`task::EVAL`].
    pub split: String,
    /// The openbatrangs program.
    pub openbatrangs: PathBuf,
    /// Where records are appended.
    pub out: PathBuf,
    /// Arguments passed to openbatrangs before its own.
    pub passed_on: Vec<String>,
    /// How long the agent may take on one task.
    pub timeout: Duration,
    /// How long the check may take.
    pub check_timeout: Duration,
    /// Where repositories are cloned from, with `{repo}` for `OWNER/NAME`.
    pub clone_url: String,
}

/// One agent run: one line of the runs file, in the format the reinforcer's
/// runs tab reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// `TASK_ID@UNIX_MILLISECONDS`, unique per run; the id slop flags name.
    pub run_id: String,
    /// The task's id.
    pub task_id: String,
    /// The task's repository.
    pub repo: String,
    /// The task's split.
    pub split: String,
    /// The task's prompt.
    pub prompt: String,
    /// True when the check passed after the agent finished.
    pub passed: bool,
    /// The agent's steps, from its transcript; 0 without one.
    pub steps: u64,
    /// `answered`, `step_limit` or `error` from the transcript, or `timeout`
    /// or `error` when there is no transcript.
    pub outcome: String,
    /// Wall-clock seconds the agent ran.
    pub wall_seconds: f64,
    /// What openbatrangs wrote to `--transcript-out`, or null.
    pub transcript: Value,
}

/// The task id of a record already written.
#[derive(Deserialize)]
struct Done {
    task_id: String,
}

/// Executes `lasso agent run`: runs every task of the chosen split that has no
/// record in `--out`, in file order, one at a time, and appends one
/// [`RunRecord`] per task. Writes a summary line and one line per task to
/// `out`.
///
/// # Errors
///
/// [`DistillError::Usage`] for options that can't be read, and any error of
/// reading the tasks, preparing a checkout or writing a record. Records
/// written before the error stay in the file.
pub fn main_with(arguments: &Arguments, out: &mut impl Write) -> Result<(), DistillError> {
    let options = RunOptions::read(arguments)?;
    let tasks: Vec<Task> = jsonl::read(&options.tasks)?;
    let done: Vec<Done> = jsonl::read_if_present(&options.out)?;
    let done: HashSet<String> = done.into_iter().map(|record| record.task_id).collect();
    let in_split: Vec<&Task> = tasks
        .iter()
        .filter(|task| task.split == options.split)
        .collect();
    let left: Vec<&Task> = in_split
        .iter()
        .copied()
        .filter(|task| !done.contains(&task.id))
        .collect();

    writeln!(
        out,
        "{} {} tasks, {} already in {}, {} left",
        in_split.len(),
        options.split,
        in_split.len() - left.len(),
        options.out.display(),
        left.len()
    )
    .map_err(DistillError::io("standard output"))?;

    left.into_iter()
        .try_for_each(|task| run_and_record(&options, task, out))
}

impl RunOptions {
    /// Parses the options of `lasso agent run`. A relative `--openbatrangs`
    /// path is made absolute.
    ///
    /// # Errors
    ///
    /// [`DistillError::Usage`] for an unknown or missing option, a split that
    /// is not `train` or `eval`, or a time limit that is not a number.
    pub fn read(arguments: &Arguments) -> Result<Self, DistillError> {
        arguments.allow_only(&[
            "tasks",
            "split",
            "openbatrangs",
            "out",
            "timeout",
            "check-timeout",
            "clone-url",
        ])?;

        Ok(RunOptions {
            tasks: PathBuf::from(arguments.required("tasks")?),
            split: split(&arguments.required("split")?)?,
            openbatrangs: program_path(&arguments.required("openbatrangs")?)?,
            out: PathBuf::from(arguments.required("out")?),
            passed_on: arguments.passed_on.clone(),
            timeout: Duration::from_secs(arguments.number("timeout", AGENT_SECONDS)?),
            check_timeout: Duration::from_secs(arguments.number("check-timeout", CHECK_SECONDS)?),
            clone_url: arguments.or("clone-url", CLONE_URL),
        })
    }
}

/// Returns `program` unchanged when it is a bare name, to be found on `PATH`,
/// and as an absolute path otherwise, since the agent's working folder is the
/// checkout.
///
/// # Errors
///
/// [`DistillError::Io`] when the current folder can't be read.
fn program_path(program: &str) -> Result<PathBuf, DistillError> {
    let path = PathBuf::from(program);

    if path.components().count() < 2 {
        return Ok(path);
    }

    std::path::absolute(&path).map_err(DistillError::io(&path))
}

/// Returns `name` when it is [`task::TRAIN`] or [`task::EVAL`].
///
/// # Errors
///
/// [`DistillError::Usage`] for any other name.
fn split(name: &str) -> Result<String, DistillError> {
    [task::TRAIN, task::EVAL]
        .contains(&name)
        .then(|| name.to_string())
        .ok_or_else(|| DistillError::Usage(format!("--split is train or eval, not {name}")))
}

/// Runs `task`, appends its record to `--out`, and writes one line about it
/// to `out`.
fn run_and_record(
    options: &RunOptions,
    task: &Task,
    out: &mut impl Write,
) -> Result<(), DistillError> {
    let record = run_one(options, task)?;
    let verdict = if record.passed { "passed" } else { "failed" };

    jsonl::append(&options.out, &record)?;
    writeln!(
        out,
        "{}: {verdict}, {} steps, {}, {:.0} s",
        record.task_id, record.steps, record.outcome, record.wall_seconds
    )
    .map_err(DistillError::io("standard output"))
}

/// Returns the record of one run of the agent on `task`. The checkout lives
/// in a temporary folder that is removed before this returns. `passed` is
/// true only when the check exits with success within its time limit.
///
/// # Errors
///
/// Any error of preparing the checkout or of starting the agent or the
/// check. An agent that fails or runs out of time is not an error; the
/// record says so.
pub fn run_one(options: &RunOptions, task: &Task) -> Result<RunRecord, DistillError> {
    let folder = Scratch::new()?;
    let workspace = folder.path.join("workspace");
    let transcript_path = folder.path.join("transcript.json");

    let test_files = prepare(task, &options.clone_url, &workspace)?;

    let started = Instant::now();
    let mut agent = agent_command(options, task, &workspace, &transcript_path);
    let finish = process::finish_within(&mut agent, options.timeout)?;
    let wall_seconds = started.elapsed().as_secs_f64();

    write_files(&test_files)?;

    let mut check = check_command(task, &workspace);
    let checked = process::finish_within(&mut check, options.check_timeout)?;
    let transcript = read_transcript(&transcript_path);

    Ok(RunRecord {
        run_id: format!("{}@{}", task.id, unix_millis()),
        task_id: task.id.clone(),
        repo: task.repo.clone(),
        split: task.split.clone(),
        prompt: task.prompt.clone(),
        passed: checked == Finish::Exited(true),
        steps: transcript.get("steps").and_then(Value::as_u64).unwrap_or(0),
        outcome: outcome(&transcript, finish),
        wall_seconds,
        transcript,
    })
}

/// Returns the transcript's `outcome`; without one, `timeout` when the agent
/// was stopped at its time limit and `error` when it exited.
fn outcome(transcript: &Value, finish: Finish) -> String {
    let ended = match finish {
        Finish::TimedOut => "timeout",
        Finish::Exited(_) => "error",
    };

    transcript
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or(ended)
        .to_string()
}

/// Returns the JSON at `path`, or null when the file is missing or is not
/// JSON.
fn read_transcript(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

/// Returns the command `openbatrangs PASSED_ON --workspace DIR
/// --transcript-out FILE PROMPT`, run in the checkout with no standard input
/// or output.
fn agent_command(
    options: &RunOptions,
    task: &Task,
    workspace: &Path,
    transcript: &Path,
) -> Command {
    let mut command = Command::new(&options.openbatrangs);

    command
        .args(&options.passed_on)
        .arg("--workspace")
        .arg(workspace)
        .arg("--transcript-out")
        .arg(transcript)
        .arg(&task.prompt)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null());

    command
}

/// Returns the command that runs the task's check with `sh -c` in the
/// checkout, with no standard input or output.
fn check_command(task: &Task, workspace: &Path) -> Command {
    let mut command = Command::new("sh");

    command
        .arg("-c")
        .arg(&task.check)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    command
}

/// A file's path and its contents.
pub type FileContents = (PathBuf, Vec<u8>);

/// Creates `workspace` as a checkout of `task.base` from `clone_url`, with
/// `{repo}` replaced by the task's repository, adds the task's test files as
/// they are at `task.merge`, and replaces the history with a single commit of
/// that state. Returns the test files' paths and contents.
///
/// # Errors
///
/// [`DistillError::Command`] when a `git` command fails, and
/// [`DistillError::Io`] when a file can't be written.
pub fn prepare(
    task: &Task,
    clone_url: &str,
    workspace: &Path,
) -> Result<Vec<FileContents>, DistillError> {
    let url = clone_url.replace("{repo}", &task.repo);
    let target = workspace.to_string_lossy();

    git(Path::new("."), &["clone", "--quiet", &url, &target])?;
    git(workspace, &["checkout", "--quiet", &task.base])?;

    let test_files: Vec<FileContents> = task
        .test_files
        .iter()
        .map(|file| test_file(workspace, &task.merge, file))
        .collect::<Result<_, _>>()?;
    write_files(&test_files)?;

    let history = workspace.join(".git");
    fs::remove_dir_all(&history).map_err(DistillError::io(&history))?;
    git(workspace, &["init", "--quiet"])?;
    git(workspace, &["add", "--all"])?;
    git(workspace, START_COMMIT)?;

    Ok(test_files)
}

/// Returns the path of `file` in `workspace` and its contents at commit
/// `merge`.
///
/// # Errors
///
/// [`DistillError::Command`] when the commit has no such file.
fn test_file(workspace: &Path, merge: &str, file: &str) -> Result<FileContents, DistillError> {
    let contents = git(workspace, &["show", &format!("{merge}:{file}")])?;

    Ok((workspace.join(file), contents))
}

/// Writes each file, creating missing folders.
///
/// # Errors
///
/// [`DistillError::Io`] for the first file that can't be written.
fn write_files(files: &[FileContents]) -> Result<(), DistillError> {
    files.iter().try_for_each(|(path, contents)| {
        let folder = path.parent().unwrap_or(path);

        fs::create_dir_all(folder).map_err(DistillError::io(folder))?;
        fs::write(path, contents).map_err(DistillError::io(path))
    })
}

/// Runs `git -C folder ARGUMENTS` and returns its standard output.
///
/// # Errors
///
/// As [`process::output`].
fn git(folder: &Path, arguments: &[&str]) -> Result<Vec<u8>, DistillError> {
    process::output(Command::new("git").arg("-C").arg(folder).args(arguments))
}

/// Returns the time since 1970-01-01 UTC, or zero for a clock set before it.
fn since_epoch() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

/// Returns the milliseconds since 1970-01-01 UTC.
fn unix_millis() -> u128 {
    since_epoch().as_millis()
}

/// A temporary folder. Dropping it removes the folder and its contents; a
/// failed removal is reported on standard error.
pub struct Scratch {
    /// The folder.
    pub path: PathBuf,
}

impl Scratch {
    /// Creates an empty folder under the system's temporary folder (`TMPDIR`),
    /// named after the process id and the current time.
    ///
    /// # Errors
    ///
    /// [`DistillError::Io`] when it can't be made.
    pub fn new() -> Result<Self, DistillError> {
        let nanos = since_epoch().as_nanos();
        let name = format!("lasso-agent-{}-{nanos}", std::process::id());
        let path = std::env::temp_dir().join(name);

        fs::create_dir_all(&path).map_err(DistillError::io(&path))?;

        Ok(Scratch { path })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            eprintln!("lasso: could not remove {}: {error}", self.path.display());
        }
    }
}
