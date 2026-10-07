//! Runs `reinforcer` as a program: the scan and apply commands, a bad
//! command line, and a serve with nothing to show.

use std::{fmt, fs, io, path::PathBuf, process::Command};

/// Why a test could not run the program.
#[derive(Debug)]
enum RunError {
    /// Writing a file or starting the program failed.
    Io(io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<io::Error> for RunError {
    fn from(error: io::Error) -> Self {
        RunError::Io(error)
    }
}

fn folder() -> Result<PathBuf, RunError> {
    let folder = std::env::temp_dir().join(format!("thor-reinforcer-bin-{}", std::process::id()));
    fs::create_dir_all(&folder)?;
    Ok(folder)
}

#[test]
fn scans_applies_and_refuses() -> Result<(), RunError> {
    let folder = folder()?;
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
    assert_eq!(RunError::from(io::Error::other("x")).to_string(), "x");
    fs::remove_dir_all(&folder)?;
    Ok(())
}
