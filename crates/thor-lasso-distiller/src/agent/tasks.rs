//! `lasso agent tasks`: one task per merged pull request that closes an issue
//! and changes test files, from `gh` output.

use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use serde::de::DeserializeOwned;

use super::{
    arguments::Arguments,
    jsonl, process,
    task::{self, Commit, Issue, PullRequest, Task},
};
use crate::error::DistillError;

/// The largest number of pull requests, and of issues, read per repository.
const LIST_LIMIT: &str = "1000";

/// A `gh` program and the repository it queries.
pub struct GitHub {
    /// The `gh` program.
    pub program: PathBuf,
    /// `OWNER/NAME`.
    pub repo: String,
}

impl GitHub {
    /// Runs `gh ARGUMENTS` and parses its standard output as a `T`.
    ///
    /// # Errors
    ///
    /// [`DistillError::Command`] when `gh` fails, and [`DistillError::Json`]
    /// when its output is not a `T`.
    pub fn json<T: DeserializeOwned>(&self, arguments: &[&str]) -> Result<T, DistillError> {
        let stdout = process::output(Command::new(&self.program).args(arguments))?;
        let what = format!("gh {}", arguments.join(" "));

        serde_json::from_slice(&stdout).map_err(DistillError::json(what, 1))
    }

    /// Returns the merged pull requests, newest first.
    ///
    /// # Errors
    ///
    /// As [`GitHub::json`].
    pub fn merged_pulls(&self) -> Result<Vec<PullRequest>, DistillError> {
        self.json(&[
            "pr",
            "list",
            "--repo",
            &self.repo,
            "--state",
            "merged",
            "--limit",
            LIST_LIMIT,
            "--json",
            "number,closingIssuesReferences,files,mergeCommit",
        ])
    }

    /// Returns every issue, open or closed, by number.
    ///
    /// # Errors
    ///
    /// As [`GitHub::json`].
    pub fn issues(&self) -> Result<HashMap<u64, Issue>, DistillError> {
        let issues: Vec<Issue> = self.json(&[
            "issue",
            "list",
            "--repo",
            &self.repo,
            "--state",
            "all",
            "--limit",
            LIST_LIMIT,
            "--json",
            "number,title,body",
        ])?;

        Ok(issues
            .into_iter()
            .map(|issue| (issue.number, issue))
            .collect())
    }

    /// Returns the first parent of commit `sha`.
    ///
    /// # Errors
    ///
    /// As [`GitHub::json`], and [`DistillError::Command`] for a commit with
    /// no parent.
    pub fn first_parent(&self, sha: &str) -> Result<String, DistillError> {
        let path = format!("repos/{}/commits/{sha}", self.repo);
        let commit: Commit = self.json(&["api", &path])?;
        let parent = commit.parents.into_iter().next();

        parent
            .map(|parent| parent.sha)
            .ok_or_else(|| DistillError::Command(format!("commit {sha} has no parent")))
    }
}

/// Executes `lasso agent tasks`: writes the tasks to `--out`, replacing its
/// contents, and one summary line to `out`.
///
/// # Errors
///
/// [`DistillError::Usage`] for missing options, and any error of `gh` or of
/// writing the tasks.
pub fn main_with(arguments: &Arguments, out: &mut impl Write) -> Result<(), DistillError> {
    arguments.allow_only(&["repo", "out", "gh"])?;

    let github = GitHub {
        program: PathBuf::from(arguments.or("gh", "gh")),
        repo: arguments.required("repo")?,
    };
    let path = PathBuf::from(arguments.required("out")?);
    let pulls = github.merged_pulls()?;
    let tasks = collect(&github, &pulls)?;

    jsonl::write(&path, &tasks)?;
    report(&path, pulls.len(), &tasks, out)
}

/// Returns one task per pull request in `pulls` that yields a [`task::Draft`].
/// When two pull requests close the same first issue, only the first in
/// `pulls` is kept.
///
/// # Errors
///
/// Any error of `gh`.
pub fn collect(github: &GitHub, pulls: &[PullRequest]) -> Result<Vec<Task>, DistillError> {
    let issues = github.issues()?;
    let mut seen = HashSet::new();

    pulls
        .iter()
        .filter_map(|pull| task::draft(pull, &issues))
        .filter(|draft| seen.insert(draft.issue))
        .map(|draft| {
            let base = github.first_parent(&draft.merge)?;

            Ok(Task::new(&github.repo, draft, base))
        })
        .collect()
}

/// Writes the task count per split and the number of merged pull requests.
fn report(
    path: &Path,
    pulls: usize,
    tasks: &[Task],
    out: &mut impl Write,
) -> Result<(), DistillError> {
    let eval = tasks.iter().filter(|task| task.split == task::EVAL).count();

    writeln!(
        out,
        "{} tasks ({} train, {eval} eval) from {pulls} merged pull requests, written to {}",
        tasks.len(),
        tasks.len() - eval,
        path.display()
    )
    .map_err(DistillError::io("standard output"))
}
