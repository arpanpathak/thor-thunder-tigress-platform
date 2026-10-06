//! Builds `data/train.jsonl` and `data/stats.md`.
//!
//! ```text
//! cargo run --release -p thor-hammer-trainer -- [OUTPUT_DIR]
//! ```

use std::{path::PathBuf, process::ExitCode};

use thor_hammer_trainer::build::{self, Inputs};

/// Builds the training set, reporting the failure reason on error.
fn main() -> ExitCode {
    let output_dir = std::env::args().nth(1).unwrap_or_else(|| "data".to_string());
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    match build::build_training_set(&Inputs::under_home(&home), &PathBuf::from(output_dir)) {
        Ok(built) => {
            println!("open corpus:");
            built.corpus_lines.iter().for_each(|line| println!("{line}"));
            built.warnings.iter().for_each(|warning| eprintln!("{warning}"));
            print!("{}", built.report);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("thor-hammer-trainer: {error}");
            ExitCode::FAILURE
        }
    }
}
