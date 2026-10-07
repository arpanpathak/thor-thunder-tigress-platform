//! `thor-tigress-keyring`: the encrypted registry of the people allowed to use
//! the Thor Tigress Cub. See [`thor_tigress_keyring::cli`] for the commands.

use std::process::ExitCode;

use thor_tigress_keyring::{cli, error::KeyringError};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let passphrase = std::env::var("THOR_KEYRING_PASSPHRASE").ok();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut prompt = std::io::stderr();
    let mut out = std::io::stdout();

    match cli::main_with(&arguments, passphrase, &mut input, &mut prompt, &mut out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let code = if matches!(error, KeyringError::Usage(_)) {
                2
            } else {
                1
            };
            eprintln!("thor-tigress-keyring: {error}");
            ExitCode::from(code)
        }
    }
}
