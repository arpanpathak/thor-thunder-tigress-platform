//! `teacher`: checks the teacher's conversations, or queues sections to write
//! new ones from. See [`thor_hammer_trainer::teacher::build`].

use std::{path::PathBuf, process::ExitCode};

use thor_hammer_trainer::{
    error::DataError,
    teacher::build::{self, Command, Paths, USAGE},
};

/// Checks the entries and prints the summary. `Ok(false)` when an entry failed.
fn check(source: PathBuf, output: PathBuf) -> Result<bool, DataError> {
    let result = build::check(&Paths::new(source, output.clone()))?;

    print!("{}", result.summary());

    if !result.all_passed {
        eprintln!(
            "teacher: some entries failed; see {}",
            output.join("teacher.md").display()
        );
    }

    Ok(result.all_passed)
}

/// Queues sections for the teacher and prints how many.
fn pick(count: usize, sources: &[String]) -> Result<bool, DataError> {
    let paths = Paths::new("train/teacher".into(), "data".into());

    print!("{}", build::queue(&paths, count, sources)?);

    Ok(true)
}

fn main() -> ExitCode {
    let outcome = match Command::from_args(std::env::args().skip(1)) {
        Command::Check { source, output } => check(source, output),
        Command::Pick { count, sources } => pick(count, &sources),
        Command::Usage => {
            eprintln!("{USAGE}");

            Ok(false)
        }
    };

    match outcome {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("teacher: {error}");

            ExitCode::FAILURE
        }
    }
}
