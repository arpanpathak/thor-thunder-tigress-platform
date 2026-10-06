//! `reinforcer`: shows the training set to a person and records slop flags.
//!
//! The whole corpus is never held in memory: [`index`] keeps only each
//! record's byte offset, and a page is answered by reading the bytes it shows.
//!
//! | Module | Does |
//! |---|---|
//! | [`cli`] | the command line: serve, scan, apply |
//! | [`workspace`] | the datasets one server shows |
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
mod workspace;

use std::{
    io,
    net::{TcpListener, TcpStream},
    path::Path,
    process::ExitCode,
    sync::Arc,
    thread,
};

use crate::{
    cli::{Command, USAGE},
    error::{Outcome, ReviewError},
    workspace::{DatasetSpec, Workspace},
};

fn main() -> ExitCode {
    let outcome = match Command::from_args(std::env::args().skip(1)) {
        Command::Serve { port, datasets } => serve(&port, &datasets),
        Command::Scan { training, out } => scan::run(&training, &out),
        Command::Apply { suggestions, flags } => scan::apply(&suggestions, &flags),
        Command::Usage(reason) => Err(ReviewError::BadRequest(format!("{reason}\n{USAGE}"))),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("reinforcer: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Opens the datasets, binds `127.0.0.1:port`, and answers each connection on its own thread.
fn serve(port: &str, datasets: &[DatasetSpec]) -> Outcome {
    let workspace = Arc::new(Workspace::open(datasets)?);
    for info in workspace.infos()? {
        println!("{:<14} {:>7} records, {:>5} flagged  {}", info.name, info.records, info.flagged, info.file);
    }
    for missing in workspace.missing() {
        println!("not found, skipped: {missing}");
    }
    let address = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&address).map_err(ReviewError::io(Path::new(&address)))?;
    println!("open http://{address}");
    accept(listener.incoming(), &workspace);
    Ok(())
}

/// Answers each connection on its own thread, until `connections` ends.
fn accept(connections: impl Iterator<Item = io::Result<TcpStream>>, workspace: &Arc<Workspace>) {
    for stream in connections.filter_map(Result::ok) {
        let workspace = Arc::clone(workspace);
        thread::spawn(move || handle(stream, &workspace));
    }
}

/// Answers one connection; a failure is logged and, when someone is still
/// listening, answered with its status.
fn handle(mut stream: TcpStream, workspace: &Workspace) {
    let outcome = http::read_request(&stream).and_then(|request| api::answer(&mut stream, &request, workspace));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use std::io::{Read, Write};

    fn exchange(address: &str, request: &str) -> Outcome<String> {
        let mut stream = TcpStream::connect(address).map_err(ReviewError::io(Path::new(address)))?;
        stream.write_all(request.as_bytes()).map_err(ReviewError::io(Path::new(address)))?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply).map_err(ReviewError::io(Path::new(address)))?;
        Ok(reply)
    }

    #[test]
    fn serve_opens_the_datasets_and_listens() -> Outcome {
        let folder = TempDir::new()?;
        let records = folder.file("train.jsonl", "{\"id\":\"a\",\"source\":\"chat\",\"origin\":\"c\"}\n")?;
        let missing = folder.path().join("none.jsonl");
        let specs = vec![
            DatasetSpec::new("train", &records, &folder.path().join("f.jsonl"), &folder.path().join("r.jsonl")),
            DatasetSpec::new("teacher", &missing, &folder.path().join("g.jsonl"), &folder.path().join("s.jsonl")),
        ];
        thread::spawn(move || serve("0", &specs));
        thread::sleep(std::time::Duration::from_millis(200));
        assert!(matches!(serve("0", &[]), Err(ReviewError::NotFound(_))));
        Ok(())
    }

    #[test]
    fn serves_every_dataset_and_reports_bad_requests() -> Outcome {
        let folder = TempDir::new()?;
        let records = folder.file("train.jsonl", "{\"id\":\"a\",\"source\":\"chat\",\"origin\":\"c\",\"instruction\":\"Q\",\"response\":\"A\"}\n")?;
        let spec = DatasetSpec::new("train", &records, &folder.path().join("flags.jsonl"), &folder.path().join("removed.jsonl"));
        let workspace = Arc::new(Workspace::open(&[spec])?);
        let listener = TcpListener::bind("127.0.0.1:0").map_err(ReviewError::io(Path::new("listener")))?;
        let address = listener.local_addr().map_err(ReviewError::io(Path::new("listener")))?.to_string();
        let server = thread::spawn(move || accept(listener.incoming().take(3), &workspace));
        let datasets = exchange(&address, "GET /api/datasets HTTP/1.1\r\n\r\n")?;
        let unknown = exchange(&address, "GET /api/meta?dataset=nope HTTP/1.1\r\n\r\n")?;
        let broken = exchange(&address, "\r\n\r\n")?;
        server.join().map_err(|_| ReviewError::Poisoned)?;
        assert!(datasets.contains("\"name\":\"train\""), "{datasets}");
        assert!(unknown.starts_with("HTTP/1.1 404"), "{unknown}");
        assert!(broken.starts_with("HTTP/1.1 4"), "{broken}");
        Ok(())
    }
}
