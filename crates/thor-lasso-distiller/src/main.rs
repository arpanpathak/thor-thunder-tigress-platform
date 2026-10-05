//! `lasso`: builds conversations from the book passages of the training set.
//!
//! ```text
//! lasso conversations [--train data/train.jsonl] [--out data/conversations.jsonl]
//!                     [--books trpl,rbe,...] [--turns 3] [--limit N]
//!                     [--server 127.0.0.1:8000] [--model NAME] [--dry-run]
//! ```
//!
//! `--dry-run` calls no model: it prints how many conversations and turns the
//! books give, and writes the first prompts to `OUT.prompts.jsonl` so they can
//! be read before any generation is paid for. Without it, every conversation
//! is appended to `--out` as soon as it is done, so a long run that stops
//! keeps its work.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::ExitCode,
    time::Instant,
};

use serde_json::json;
use thor_lasso_distiller::{
    client::Client,
    conversations::{self, Passage},
    error::DistillError,
};

/// The command line of `lasso conversations`.
struct Options {
    train: PathBuf,
    out: PathBuf,
    books: Vec<String>,
    turns: usize,
    limit: Option<usize>,
    client: Client,
    dry_run: bool,
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match arguments.split_first() {
        Some((command, rest)) if command == "conversations" => options(rest).and_then(|options| run(&options)),
        _ => Err(DistillError::Usage(usage())),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lasso: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> String {
    "usage: lasso conversations [--train FILE] [--out FILE] [--books a,b] [--turns N] \
     [--limit N] [--server HOST:PORT] [--model NAME] [--dry-run]"
        .to_string()
}

fn options(arguments: &[String]) -> Result<Options, DistillError> {
    let mut options = Options {
        train: PathBuf::from("data/train.jsonl"),
        out: PathBuf::from("data/conversations.jsonl"),
        books: Vec::new(),
        turns: 3,
        limit: None,
        client: Client {
            address: "127.0.0.1:8000".to_string(),
            model: "teacher".to_string(),
        },
        dry_run: false,
    };
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        let mut value = || {
            rest.next()
                .cloned()
                .ok_or_else(|| DistillError::Usage(format!("{argument} needs a value")))
        };
        let number = |text: String| {
            text.parse::<usize>()
                .map_err(|_| DistillError::Usage(format!("{argument} needs a number, not {text}")))
        };
        match argument.as_str() {
            "--train" => options.train = PathBuf::from(value()?),
            "--out" => options.out = PathBuf::from(value()?),
            "--books" => options.books = value()?.split(',').map(str::to_string).collect(),
            "--turns" => options.turns = number(value()?)?,
            "--limit" => options.limit = Some(number(value()?)?),
            "--server" => options.client.address = value()?,
            "--model" => options.client.model = value()?,
            "--dry-run" => options.dry_run = true,
            other => return Err(DistillError::Usage(format!("unknown option {other}\n{}", usage()))),
        }
    }
    Ok(options)
}

fn run(options: &Options) -> Result<(), DistillError> {
    let passages = conversations::read_passages(&options.train, &options.books)?;
    let planned = conversations::plan(passages, options.turns);
    let counts = conversations::plan_counts(&planned);
    let turns: usize = planned.iter().map(Vec::len).sum();
    println!("{} conversations, {turns} turns, from {} books and doc sets", planned.len(), counts.len());
    counts
        .iter()
        .for_each(|(book, (talks, turns))| println!("  {book:<44} {talks:>5} conversations {turns:>6} turns"));
    let chosen: Vec<&Vec<Passage>> = planned.iter().take(options.limit.unwrap_or(planned.len())).collect();
    match options.dry_run {
        true => write_prompts(options, &chosen),
        false => generate(options, &chosen),
    }
}

fn write_prompts(options: &Options, chosen: &[&Vec<Passage>]) -> Result<(), DistillError> {
    let path = options.out.with_extension("prompts.jsonl");
    let lines: Vec<String> = chosen
        .iter()
        .take(options.limit.unwrap_or(20))
        .filter_map(|conversation| conversation.first())
        .map(|passage| {
            let messages: Vec<serde_json::Value> = conversations::question_prompt(&[], passage)
                .iter()
                .map(|message| json!({ "role": message.role, "content": message.content }))
                .collect();
            json!({ "origin": passage.origin, "messages": messages }).to_string()
        })
        .collect();
    fs::write(&path, lines.join("\n") + "\n").map_err(DistillError::io(&path))?;
    println!("dry run: wrote {} first-turn prompts to {}", lines.len(), path.display());
    Ok(())
}

fn generate(options: &Options, chosen: &[&Vec<Passage>]) -> Result<(), DistillError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&options.out)
        .map_err(DistillError::io(&options.out))?;
    let started = Instant::now();
    let (mut kept, mut rejected) = (0usize, 0usize);
    for (done, sections) in chosen.iter().enumerate() {
        let conversation = conversations::converse(&options.client, sections)?;
        kept += conversation.turns.len();
        rejected += conversation.rejected.len();
        if !conversation.turns.is_empty() {
            writeln!(file, "{}", conversation.to_json(&options.client.model)).map_err(DistillError::io(&options.out))?;
        }
        if (done + 1) % 10 == 0 || done + 1 == chosen.len() {
            println!(
                "{}/{} conversations, {kept} questions kept, {rejected} rejected, {:.0} s",
                done + 1,
                chosen.len(),
                started.elapsed().as_secs_f64()
            );
        }
    }
    println!("wrote {}", options.out.display());
    Ok(())
}
