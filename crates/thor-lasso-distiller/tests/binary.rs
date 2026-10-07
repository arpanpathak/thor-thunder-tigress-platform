//! Runs `lasso` as a program.

use std::{fs, path::Path, process::Command};

use thor_lasso_distiller::error::DistillError;

#[test]
fn a_dry_run_succeeds_and_a_bad_command_fails() -> Result<(), DistillError> {
    let folder = std::env::temp_dir().join(format!("thor-lasso-bin-{}", std::process::id()));
    fs::create_dir_all(&folder).map_err(DistillError::io(&folder))?;
    let train = folder.join("train.jsonl");
    fs::write(
        &train,
        r##"{"instruction":"","response":"# Vectors\n\nUse push.","origin":"trpl/src/ch08.md"}"##,
    )
    .map_err(DistillError::io(&train))?;
    let out = folder.join("conversations.jsonl");
    let program = Path::new(env!("CARGO_BIN_EXE_lasso"));
    let dry = Command::new(program)
        .args(["conversations", "--dry-run", "--train"])
        .arg(&train)
        .arg("--out")
        .arg(&out)
        .output()
        .map_err(DistillError::io(program))?;
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(String::from_utf8_lossy(&dry.stdout).contains("dry run: wrote 1 first-turn prompts"));
    let bad = Command::new(program)
        .arg("help")
        .output()
        .map_err(DistillError::io(program))?;
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).starts_with("lasso: usage: lasso conversations"));
    fs::remove_dir_all(&folder).map_err(DistillError::io(&folder))
}
