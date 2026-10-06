//! `spark`: the Stage 0 command line. The commands live in
//! [`thor_spark_safety_eval::cli`]; this prints their report and turns how
//! they ended into the exit status: 0 clean, 1 problems found, 2 an error.

use std::process::ExitCode;

use thor_spark_safety_eval::cli::{Command, Exit};

/// The exit status for an error, as opposed to problems found.
const ERROR_STATUS: u8 = 2;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match Command::from_args(&arguments).and_then(|command| command.run()) {
        Ok(report) => {
            for line in &report.lines {
                println!("{line}");
            }
            match report.exit {
                Exit::Clean => ExitCode::SUCCESS,
                Exit::Problems => ExitCode::FAILURE,
            }
        }
        Err(error) => {
            eprintln!("spark: {error}");
            ExitCode::from(ERROR_STATUS)
        }
    }
}
