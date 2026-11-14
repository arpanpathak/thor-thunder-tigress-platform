//! Coding tasks from merged pull requests, and the rules that select them
//! from `gh` output.
//!
//! A task's prompt is the text of the issues its pull request closes, as the
//! person who asked for the change wrote it. The agent starts from the commit
//! the pull request was merged onto, with the pull request's test files
//! added. The task is solved when the check command exits with success.

use std::{collections::HashMap, path::Path};

use serde::{Deserialize, Serialize};

/// The split of tasks used for training.
pub const TRAIN: &str = "train";

/// The held-out split, about one task in [`EVAL_ONE_IN`].
pub const EVAL: &str = "eval";

/// The ratio of all tasks to [`EVAL`] tasks.
pub const EVAL_ONE_IN: u64 = 5;

/// The check command for each test-file extension. Test files with other
/// extensions are ignored.
pub const CHECKS: &[(&str, &str)] = &[("rs", "cargo test")];

/// A coding task: one line of the tasks file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// `OWNER/NAME#ISSUE`.
    pub id: String,
    /// `OWNER/NAME`.
    pub repo: String,
    /// The first issue the pull request closes.
    pub issue: u64,
    /// The pull request.
    pub pr: u64,
    /// The title and body of each issue the pull request closes.
    pub prompt: String,
    /// The first parent of the merge commit; the agent starts here.
    pub base: String,
    /// The merge commit; the test files are taken from it.
    pub merge: String,
    /// The test files the pull request added or changed.
    pub test_files: Vec<String>,
    /// The shell command, run in the checkout, that exits with success when
    /// the task is solved.
    pub check: String,
    /// [`TRAIN`] or [`EVAL`].
    pub split: String,
}

/// A merged pull request, as `gh pr list --json` gives it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    /// Its number.
    pub number: u64,
    /// The issues it closes.
    pub closing_issues_references: Vec<IssueReference>,
    /// The files it changed.
    pub files: Vec<ChangedFile>,
    /// The commit that merged it.
    pub merge_commit: Option<MergeCommit>,
}

/// An issue a pull request closes.
#[derive(Debug, Clone, Deserialize)]
pub struct IssueReference {
    /// The issue's number.
    pub number: u64,
}

/// A file a pull request changed.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    /// Its path in the repository.
    pub path: String,
    /// `ADDED`, `MODIFIED`, `DELETED` or `RENAMED`.
    pub change_type: String,
}

/// The commit that merged a pull request.
#[derive(Debug, Clone, Deserialize)]
pub struct MergeCommit {
    /// Its hash.
    pub oid: String,
}

/// An issue, as `gh issue list --json` gives it.
#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    /// Its number.
    pub number: u64,
    /// Its title.
    pub title: String,
    /// Its body.
    pub body: String,
}

/// A commit, as `gh api repos/OWNER/NAME/commits/SHA` gives it.
#[derive(Debug, Clone, Deserialize)]
pub struct Commit {
    /// Its parents; the first is the branch the pull request was merged onto.
    pub parents: Vec<Parent>,
}

/// A parent of a [`Commit`].
#[derive(Debug, Clone, Deserialize)]
pub struct Parent {
    /// Its hash.
    pub sha: String,
}

/// The fields of a task that come from its pull request; the base commit
/// needs one more `gh` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// The first issue it closes.
    pub issue: u64,
    /// The pull request.
    pub pr: u64,
    /// The text of the issues it closes.
    pub prompt: String,
    /// The merge commit.
    pub merge: String,
    /// Its test files with an extension in [`CHECKS`].
    pub test_files: Vec<String>,
    /// The check command for those files.
    pub check: String,
}

/// File names, without extension, that hold only test code.
const TEST_STEMS: &[&str] = &["tests", "testing"];

/// Endings of file names, without extension, that hold only test code.
const TEST_SUFFIXES: &[&str] = &["_test", "_tests"];

/// Returns true when `path` is a test file: it is under a `tests` folder, or
/// its name without extension is in [`TEST_STEMS`] or ends in one of
/// [`TEST_SUFFIXES`].
#[must_use]
pub fn is_test_file(path: &str) -> bool {
    let file = Path::new(path);
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    let in_tests = file
        .components()
        .rev()
        .skip(1)
        .any(|part| part.as_os_str() == "tests");
    let named = TEST_STEMS.contains(&stem);
    let suffixed = TEST_SUFFIXES.iter().any(|suffix| stem.ends_with(suffix));

    [in_tests, named, suffixed].contains(&true)
}

/// Returns the check command for the test file at `path` from [`CHECKS`], or
/// `None` for an extension not listed there.
#[must_use]
pub fn check_for(path: &str) -> Option<&'static str> {
    let extension = Path::new(path).extension()?.to_str()?;

    CHECKS
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|(_, check)| *check)
}

/// Returns the test files `pull` added or changed, without deleted files and
/// files with no check command, and the check command of the first. Returns
/// `None` when no file remains.
#[must_use]
pub fn test_files(pull: &PullRequest) -> Option<(Vec<String>, String)> {
    let files: Vec<&ChangedFile> = pull
        .files
        .iter()
        .filter(|file| file.change_type != "DELETED")
        .filter(|file| is_test_file(&file.path))
        .filter(|file| check_for(&file.path).is_some())
        .collect();
    let check = check_for(&files.first()?.path)?;

    Some((
        files.iter().map(|file| file.path.clone()).collect(),
        check.to_string(),
    ))
}

/// Returns the prompt for `pull`: the title and body of each issue it closes
/// that `issues` holds, separated by blank lines. Returns `None` when there is
/// no such issue.
#[must_use]
pub fn prompt(pull: &PullRequest, issues: &HashMap<u64, Issue>) -> Option<String> {
    let parts: Vec<String> = pull
        .closing_issues_references
        .iter()
        .filter_map(|reference| issues.get(&reference.number))
        .map(|issue| format!("{}\n\n{}", issue.title.trim(), issue.body.trim()))
        .collect();

    Some(parts.join("\n\n")).filter(|text| !text.is_empty())
}

/// Returns the [`Draft`] for `pull`, or `None` when it changes no test file,
/// closes no issue in `issues`, or has no merge commit.
#[must_use]
pub fn draft(pull: &PullRequest, issues: &HashMap<u64, Issue>) -> Option<Draft> {
    let (test_files, check) = test_files(pull)?;

    Some(Draft {
        issue: pull.closing_issues_references.first()?.number,
        pr: pull.number,
        prompt: prompt(pull, issues)?,
        merge: pull.merge_commit.as_ref()?.oid.clone(),
        test_files,
        check,
    })
}

/// Returns [`EVAL`] when the [`fnv1a`] hash of `id` is a multiple of
/// [`EVAL_ONE_IN`], and [`TRAIN`] otherwise. The result is the same on every
/// machine and Rust version.
#[must_use]
pub fn split_of(id: &str) -> &'static str {
    if fnv1a(id).is_multiple_of(EVAL_ONE_IN) {
        return EVAL;
    }

    TRAIN
}

/// Returns the 64-bit FNV-1a hash of `text`. Unlike the standard library's
/// hasher, it is fixed across Rust versions.
#[must_use]
pub fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

impl Task {
    /// Returns the task for `draft` in `repo` with base commit `base`. Its id
    /// is `REPO#ISSUE`.
    #[must_use]
    pub fn new(repo: &str, draft: Draft, base: String) -> Self {
        let id = format!("{repo}#{}", draft.issue);

        Task {
            split: split_of(&id).to_string(),
            id,
            repo: repo.to_string(),
            issue: draft.issue,
            pr: draft.pr,
            prompt: draft.prompt,
            base,
            merge: draft.merge,
            test_files: draft.test_files,
            check: draft.check,
        }
    }
}
