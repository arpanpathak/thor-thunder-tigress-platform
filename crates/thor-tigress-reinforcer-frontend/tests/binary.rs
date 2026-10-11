//! Runs `reinforcer` as a program: the scan and apply commands, a bad
//! command line, a serve with nothing to show, and a serve stopped with Ctrl-C.

use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result};

const PROGRAM: &str = env!("CARGO_BIN_EXE_reinforcer");

/// A training file with one record whose answer is `response`.
fn train_file(folder: &Path, response: &str) -> Result<PathBuf> {
    let train = folder.join("train.jsonl");
    let record = serde_json::json!({
        "id": "a", "source": "chat", "origin": "c", "instruction": "q", "response": response
    });

    fs::write(&train, format!("{record}\n"))?;

    Ok(train)
}

fn complained(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn scans_and_applies() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let train = train_file(folder.path(), "In summary, it works.")?;
    let suggestions = folder.path().join("auto.jsonl");
    let flags = folder.path().join("flags.jsonl");
    let scanned = Command::new(PROGRAM)
        .arg("scan")
        .arg(&train)
        .arg(&suggestions)
        .output()?;
    let applied = Command::new(PROGRAM)
        .arg("apply")
        .arg(&suggestions)
        .arg(&flags)
        .output()?;

    assert!(scanned.status.success(), "{}", complained(&scanned));
    assert!(applied.status.success(), "{}", complained(&applied));
    assert!(String::from_utf8_lossy(&applied.stdout).starts_with("applied "));

    Ok(())
}

#[test]
fn refuses_an_unknown_option_and_a_serve_with_nothing_to_show() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let missing = folder.path().join("none.jsonl").display().to_string();
    let usage = Command::new(PROGRAM)
        .args(["serve", "--colour", "dark"])
        .output()?;
    let nothing = Command::new(PROGRAM)
        .args(["serve", "--dataset", &format!("x={missing}")])
        .output()?;

    assert!(!usage.status.success());
    assert!(complained(&usage).contains("usage: reinforcer"));
    assert!(!nothing.status.success());
    assert!(complained(&nothing).contains("no dataset to show"));

    Ok(())
}

#[test]
fn serves_until_interrupted() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let train = train_file(folder.path(), "")?;
    let dataset = format!("train={}", train.display());
    let mut serving = Command::new(PROGRAM)
        .args(["serve", "--port", "0", "--dataset", &dataset])
        .stdout(Stdio::piped())
        .spawn()?;
    let printed = serving.stdout.take().context("no standard output")?;
    let opened = BufReader::new(printed)
        .lines()
        .map_while(Result::ok)
        .any(|line| line.starts_with("open http://"));

    assert!(opened);

    let interrupted = Command::new("kill")
        .args(["-INT", &serving.id().to_string()])
        .status()?;

    assert!(interrupted.success());
    assert!(serving.wait()?.success());

    Ok(())
}
