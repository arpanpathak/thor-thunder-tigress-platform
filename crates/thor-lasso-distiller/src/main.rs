//! `lasso`: builds conversations from the book passages of the training set.
//! See [`thor_lasso_distiller::cli`] for the options.

use std::process::ExitCode;

use thor_lasso_distiller::cli;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let key = std::env::var("LASSO_KEY").ok();
    match cli::main_with(&arguments, key, &mut std::io::stdout()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lasso: {error}");
            ExitCode::FAILURE
        }
    }
}
