//! `reinforcer`: shows the training set to a person and records slop flags.
//!
//! The whole corpus is never held in memory: [`datasets::index`] keeps only
//! each record's byte offset, and a page is answered by reading the bytes it
//! shows.
//!
//! | Module | Does |
//! |---|---|
//! | [`cli`] | the command line: serve, scan, apply |
//! | [`server`] | the page's routes and their answers, on axum |
//! | [`datasets`] | the datasets one server shows, each training file by byte offset |
//! | [`review`] | the reviewer's flags, their categories and their file |
//! | [`suggest`] | suggestions from Stage 0, for one record or a whole file |

#![forbid(unsafe_code)]

mod cli;
mod datasets;
mod error;
mod review;
mod server;
mod suggest;
#[cfg(test)]
mod testing;

use std::{path::Path, process::ExitCode, sync::Arc};

use anyhow::bail;
use tokio::{
    net::TcpListener,
    signal::unix::{SignalKind, signal},
};

use crate::{
    cli::{Command, USAGE},
    error::{Outcome, ReviewError},
    datasets::workspace::{DatasetSpec, Workspace},
};

#[tokio::main]
async fn main() -> ExitCode {
    match run(Command::from_args(std::env::args().skip(1))).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("reinforcer: {error}");

            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Serve { port, datasets } => serve(&port, &datasets).await?,
        Command::Scan { training, out } => suggest::scan::run(&training, &out)?,
        Command::Apply { suggestions, flags } => suggest::scan::apply(&suggestions, &flags)?,
        Command::Usage(reason) => bail!("{reason}\n{USAGE}"),
    }

    Ok(())
}

/// Opens the datasets, binds `127.0.0.1:port`, and answers requests until
/// Ctrl-C; before that it returns only when opening, binding or the listener
/// fails.
async fn serve(port: &str, datasets: &[DatasetSpec]) -> Outcome {
    let workspace = Arc::new(Workspace::open(datasets)?);

    for info in workspace.infos()? {
        println!(
            "{:<14} {:>7} records, {:>5} flagged  {}",
            info.name, info.records, info.flagged, info.file
        );
    }

    for missing in workspace.missing() {
        println!("not found, skipped: {missing}");
    }

    let address = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&address)
        .await
        .map_err(ReviewError::io(Path::new(&address)))?;

    let mut interrupt =
        signal(SignalKind::interrupt()).map_err(ReviewError::io(Path::new("SIGINT")))?;

    println!("open http://{address}");

    let interrupted = async move {
        interrupt.recv().await;
    };

    server::serve(listener, workspace, interrupted).await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::testing::TempDir;

    #[tokio::test]
    async fn serve_opens_the_datasets_and_listens() -> Outcome {
        let folder = TempDir::new()?;
        let line = "{\"id\":\"a\",\"source\":\"chat\",\"origin\":\"c\"}\n";
        let records = folder.file("train.jsonl", line)?;
        let missing = folder.path().join("none.jsonl");
        let specs = vec![
            DatasetSpec::new(
                "train",
                &records,
                &folder.path().join("f.jsonl"),
                &folder.path().join("r.jsonl"),
            ),
            DatasetSpec::new(
                "teacher",
                &missing,
                &folder.path().join("g.jsonl"),
                &folder.path().join("s.jsonl"),
            ),
        ];
        let serving = tokio::spawn(async move { serve("0", &specs).await });

        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(!serving.is_finished());

        serving.abort();

        assert!(matches!(
            serve("0", &[]).await,
            Err(ReviewError::NotFound(_))
        ));

        Ok(())
    }
}
