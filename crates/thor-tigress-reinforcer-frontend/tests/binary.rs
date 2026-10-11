//! Runs `reinforcer` as a program: the scan and apply commands, a bad
//! command line, a serve with nothing to show, and a serve stopped with Ctrl-C.

use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{Context, Result};

fn folder(name: &str) -> Result<PathBuf> {
    let folder =
        std::env::temp_dir().join(format!("thor-reinforcer-{name}-{}", std::process::id()));
    fs::create_dir_all(&folder)?;

    Ok(folder)
}

#[test]
fn scans_applies_and_refuses() -> Result<()> {
    let folder = folder("bin")?;
    let train = folder.join("train.jsonl");

    fs::write(
        &train,
        "{\"id\":\"a\",\"source\":\"chat\",\"origin\":\"c\",\"instruction\":\"q\",\"response\":\"In summary, it works.\"}\n",
    )?;

    let suggestions = folder.join("auto.jsonl");
    let flags = folder.join("flags.jsonl");
    let program = env!("CARGO_BIN_EXE_reinforcer");
    let scanned = Command::new(program)
        .arg("scan")
        .arg(&train)
        .arg(&suggestions)
        .output()?;

    assert!(
        scanned.status.success(),
        "{}",
        String::from_utf8_lossy(&scanned.stderr)
    );

    let applied = Command::new(program)
        .arg("apply")
        .arg(&suggestions)
        .arg(&flags)
        .output()?;

    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert!(String::from_utf8_lossy(&applied.stdout).starts_with("applied "));

    let usage = Command::new(program)
        .args(["serve", "--colour", "dark"])
        .output()?;

    assert!(!usage.status.success());
    assert!(String::from_utf8_lossy(&usage.stderr).contains("usage: reinforcer"));

    let missing = folder.join("none.jsonl").display().to_string();
    let nothing = Command::new(program)
        .args(["serve", "--dataset", &format!("x={missing}")])
        .output()?;

    assert!(!nothing.status.success());
    assert!(String::from_utf8_lossy(&nothing.stderr).contains("no dataset to show"));

    fs::remove_dir_all(&folder)?;

    Ok(())
}

#[test]
fn serves_until_interrupted() -> Result<()> {
    let folder = folder("serve")?;
    let train = folder.join("train.jsonl");

    fs::write(
        &train,
        "{\"id\":\"a\",\"source\":\"chat\",\"origin\":\"c\"}\n",
    )?;

    let dataset = format!("train={}", train.display());
    let mut serving = Command::new(env!("CARGO_BIN_EXE_reinforcer"))
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

    fs::remove_dir_all(&folder)?;

    Ok(())
}
