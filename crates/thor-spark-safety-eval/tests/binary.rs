//! Runs `spark` as a program and checks its exit status: 0 clean, 1 problems, 2 an error.

use std::{fs, path::Path, process::Command};

use thor_spark_safety_eval::error::EvalError;

#[test]
fn exits_by_what_it_found() -> Result<(), EvalError> {
    let folder = std::env::temp_dir().join(format!("thor-spark-bin-{}", std::process::id()));
    fs::create_dir_all(&folder).map_err(EvalError::io(&folder))?;
    let clean = folder.join("clean.rs");
    fs::write(&clean, "/// One.\npub const ONE: u8 = 1;\n").map_err(EvalError::io(&clean))?;
    let broken = folder.join("broken.rs");
    fs::write(&broken, "pub fn f() { /* why */ Some(1).unwrap(); }\n").map_err(EvalError::io(&broken))?;
    let program = Path::new(env!("CARGO_BIN_EXE_spark"));
    let status = |arguments: &[&Path]| Command::new(program).arg("rs").args(arguments).output().map_err(EvalError::io(program));
    let passed = status(&[&clean])?;
    assert_eq!(passed.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&passed.stdout).contains("1 files, 0 problems"));
    assert_eq!(status(&[&broken])?.status.code(), Some(1));
    let failed = Command::new(program).arg("nonsense").output().map_err(EvalError::io(program))?;
    assert_eq!(failed.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&failed.stderr).starts_with("spark: "));
    fs::remove_dir_all(&folder).map_err(EvalError::io(&folder))
}
