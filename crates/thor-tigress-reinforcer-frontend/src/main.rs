//! `reinforcer`: shows the training set to a person and records slop flags.
//!
//! The whole corpus is never held in memory: [`index`] keeps only each
//! record's byte offset, and a page is answered by reading the bytes it shows.
//!
//! | Module | Does |
//! |---|---|
//! | [`cli`] | the command line: serve, scan, apply |
//! | [`app`] | what every connection shares |
//! | [`api`] | the page's routes and their answers |
//! | [`http`] | reading requests, writing answers |
//! | [`index`] | the training file by byte offset |
//! | [`flags`], [`category`], [`jsonl`] | the reviewer's flags and their files |
//! | [`scan`], [`stage0`], [`titles`] | suggestions and readable names |

#![forbid(unsafe_code)]

mod api;
mod app;
mod category;
mod cli;
mod error;
mod flags;
mod http;
mod index;
mod jsonl;
mod scan;
mod stage0;
#[cfg(test)]
mod testing;
mod titles;

use std::{
    net::{TcpListener, TcpStream},
    path::Path,
    process::ExitCode,
    sync::Arc,
    thread,
};

use crate::{
    app::App,
    cli::Command,
    error::{Outcome, ReviewError},
};

fn main() -> ExitCode {
    let outcome = match Command::from_args(std::env::args().skip(1)) {
        Command::Serve { training, port, flags, slop } => serve(&training, &port, &flags, &slop),
        Command::Scan { training, out } => scan::run(&training, &out),
        Command::Apply { suggestions, flags } => scan::apply(&suggestions, &flags),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("reinforcer: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Opens the files, binds `127.0.0.1:port`, and answers each connection on its own thread.
fn serve(training: &Path, port: &str, flags: &Path, slop: &Path) -> Outcome {
    let app = Arc::new(App::open(training, flags, slop)?);
    println!("indexed {} records from {}", app.index.len(), training.display());
    println!("flags are saved to {}", flags.display());
    let address = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&address).map_err(ReviewError::io(Path::new(&address)))?;
    println!("open http://{address}");
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let app = Arc::clone(&app);
                thread::spawn(move || handle(stream, &app));
            }
            Err(error) => eprintln!("reinforcer: {error}"),
        }
    }
    Ok(())
}

/// Answers one connection; a failure is logged and, when someone is still
/// listening, answered with its status.
fn handle(mut stream: TcpStream, app: &App) {
    let outcome = http::read_request(&stream).and_then(|request| api::answer(&mut stream, &request, app));
    let Err(error) = outcome else {
        return;
    };
    eprintln!("reinforcer: {error}");
    let Some(status) = error.status() else {
        return;
    };
    let body = serde_json::json!({ "error": error.to_string() });
    if let Err(unsent) = http::write_json(&mut stream, status, &body) {
        eprintln!("reinforcer: could not answer: {unsent}");
    }
}
