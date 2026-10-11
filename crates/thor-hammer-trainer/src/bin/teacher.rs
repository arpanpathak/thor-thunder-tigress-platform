//! `teacher`: checks the teacher's conversations, or queues sections to write
//! new ones from. See [`thor_hammer_trainer::teacher::build`].

use std::process::ExitCode;

use thor_hammer_trainer::teacher::build::{self, Command, Paths, USAGE};

fn main() -> ExitCode {
    let outcome = match Command::from_args(std::env::args().skip(1)) {
        Command::Check { source, output } => {
            build::check(&Paths::new(source, output.clone())).map(|result| {
                print!("{}", result.summary());

                if !result.all_passed {
                    eprintln!(
                        "teacher: some entries failed; see {}",
                        output.join("teacher.md").display()
                    );
                }

                result.all_passed
            })
        }
        Command::Pick { count, sources } => {
            let paths = Paths::new("train/teacher".into(), "data".into());

            build::queue(&paths, count, &sources).map(|summary| {
                print!("{summary}");

                true
            })
        }
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
