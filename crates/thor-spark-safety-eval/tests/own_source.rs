//! The checker run on this repository's own Rust, as the README describes: every
//! crate in the workspace has to pass the five rules it enforces.

use std::{fs, path::Path};

use thor_spark_safety_eval::{
    cli,
    error::{EvalError, Outcome},
    rules,
};

/// Fewer files than this means the walk looked in the wrong place.
const FEWEST_FILES: usize = 10;

#[test]
fn every_crate_in_the_workspace_passes_the_five_rules() -> Outcome {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let files = cli::rust_files(std::slice::from_ref(&crates))?;
    assert!(files.len() > FEWEST_FILES, "found only {} files under {}", files.len(), crates.display());
    for file in &files {
        let report = rules::check(&fs::read_to_string(file).map_err(EvalError::io(file))?);
        assert_eq!(report.parse_error, None, "{} does not parse", file.display());
        assert!(report.violations.is_empty(), "{}: {:?}", file.display(), report.violations);
    }
    Ok(())
}
