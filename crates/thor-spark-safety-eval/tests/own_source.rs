//! The checker run on this repository's own Rust, as the README describes: every
//! crate in the workspace has to pass the five rules it enforces.

use std::{
    fs,
    path::{Path, PathBuf},
};

use thor_spark_safety_eval::{error::EvalError, rules};

fn rust_files(directory: &Path) -> Result<Vec<PathBuf>, EvalError> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).map_err(EvalError::io(directory))? {
        let path = entry.map_err(EvalError::io(directory))?.path();
        let skipped = path
            .file_name()
            .is_some_and(|name| name == "target" || name.to_string_lossy().starts_with('.'));
        if skipped {
            continue;
        }
        if path.is_dir() {
            files.extend(rust_files(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(files)
}

#[test]
fn every_crate_in_the_workspace_passes_the_five_rules() -> Result<(), EvalError> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let files = rust_files(&crates)?;
    assert!(files.len() > 10, "found only {} files under {}", files.len(), crates.display());
    let mut problems = Vec::new();
    for file in &files {
        let report = rules::check(&fs::read_to_string(file).map_err(EvalError::io(file))?);
        problems.extend(report.parse_error.map(|error| format!("{}: {error}", file.display())));
        problems.extend(report.violations.iter().map(|violation| {
            format!("{}:{}: {}: {}", file.display(), violation.line, violation.rule.label(), violation.detail)
        }));
    }
    assert!(problems.is_empty(), "rule violations:\n{}", problems.join("\n"));
    Ok(())
}
