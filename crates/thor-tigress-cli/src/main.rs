//! The `thor-tigress` command. See [`thor_tigress_cli::cli::USAGE`].

use std::process::ExitCode;

use thor_tigress_cli::{
    cli::{self, Command, USAGE},
    commands,
    error::Outcome,
    machine::Machine,
    prompt::Terminal,
    serve,
};

/// Runs the command and prints any error on standard error.
fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("thor-tigress: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Reads the command line and the machine, then runs the command.
fn run(arguments: &[String]) -> Outcome<()> {
    let cli = cli::parse(arguments)?;
    let machine = Machine::load(cli.settings)?;

    match cli.command {
        Command::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Command::Engines => {
            commands::engines(&machine);
            Ok(())
        }
        Command::List => {
            commands::list(&machine);
            Ok(())
        }
        Command::Serve(options) => serve::serve(&machine, &options, &mut Terminal),
        Command::Stop { name, yes } => commands::stop(&machine, name, yes, &mut Terminal),
        Command::Logs { name } => commands::logs(name, &mut Terminal),
    }
}
