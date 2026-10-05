//! Builds `data/train.jsonl` and `data/stats.md`.
//!
//! ```text
//! cargo run --release -p thor-hammer-trainer -- [OUTPUT_DIR]
//! ```

use std::path::Path;
use std::process::ExitCode;

/// Builds the training set, reporting the failure reason on error.
fn main() -> ExitCode {
    let output_dir = std::env::args().nth(1).unwrap_or_else(|| "data".to_string());
    match thor_hammer_trainer::build::build_training_set(Path::new(&output_dir)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("thor-hammer-trainer: {error}");
            ExitCode::FAILURE
        }
    }
}
