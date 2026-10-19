//! The `spark` commands, as a library so they can be tested.
//!
//! ```text
//! spark rs    PATH...    check .rs files (directories are walked) against the five rules
//! spark text  PATH...    find slop in text or markdown files
//! spark score RUN.jsonl [--field F] [--code-field F] [--label L] [--out OUT.jsonl]
//! spark generate --url URL --model M --rules RULES.md --out RUN.jsonl
//!                [--key-file F] [--backend NAME] [--think]
//! ```
//!
//! `score` reads one answer per JSONL line from `--field` (default `text`). With
//! `--code-field`, the code is read from that field and the prose from
//! `--field`. Each line is written back with a `spark` object added, to
//! `--out` (default `RUN.scored.jsonl`) with the label in `spark_run`, and a
//! summary row is reported. `rs` and `text` end with [`Exit::Problems`] when
//! they find anything, so they can gate a build.

use std::{
    collections::BTreeMap,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{
    answer::{self, AnswerScore},
    error::{EvalError, Outcome},
    report::Summary,
    code::rules,
    generate::{self, GenerateOptions},
    prose::slop,
};

/// The JSONL field an answer is read from unless `--field` says otherwise.
const DEFAULT_FIELD: &str = "text";

/// Folders the walk never enters: build output.
const SKIPPED_FOLDERS: [&str; 1] = ["target"];

/// What `spark` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Check Rust files against the five rules.
    Rust(Vec<PathBuf>),
    /// Find slop in text files.
    Text(Vec<PathBuf>),
    /// Score every answer of an eval run.
    Score(ScoreOptions),
    /// Collect a run from a model server.
    Generate(GenerateOptions),
}

/// The options of `spark score`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreOptions {
    /// The run, one JSON answer per line.
    pub input: PathBuf,
    /// The field holding the answer, or its prose when code is separate.
    pub field: String,
    /// The field holding the code, when it is kept apart from the prose.
    pub code_field: Option<String>,
    /// The run's name in the summary and in each written line.
    pub label: String,
    /// Where the scored lines go.
    pub output: PathBuf,
}

/// How `spark` ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Nothing found.
    Clean,
    /// Something found: a broken rule, a file that doesn't parse, or slop.
    Problems,
}

/// What a command found, as lines to print, and how it ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The lines, in order.
    pub lines: Vec<String>,
    /// Clean or not.
    pub exit: Exit,
}

impl Command {
    /// Reads the arguments after the program name.
    ///
    /// # Errors
    ///
    /// `EvalError::Usage` for an unknown command, missing paths, or a bad option.
    pub fn from_args(arguments: &[String]) -> Outcome<Command> {
        let Some((command, rest)) = arguments.split_first() else {
            return Err(EvalError::Usage(usage()));
        };

        match command.as_str() {
            "rs" => Ok(Command::Rust(paths(rest)?)),
            "text" => Ok(Command::Text(paths(rest)?)),
            "score" => Ok(Command::Score(ScoreOptions::parse(rest)?)),
            "generate" => Ok(Command::Generate(generate_options(rest)?)),
            _ => Err(EvalError::Usage(usage())),
        }
    }

    /// Runs the command.
    ///
    /// # Errors
    ///
    /// `EvalError::Io` for unreadable or unwritable files, `EvalError::Json`
    /// and `EvalError::MissingField` for a malformed run.
    pub fn run(&self) -> Outcome<Report> {
        match self {
            Command::Rust(paths) => check_rust(paths),
            Command::Text(paths) => check_text(paths),
            Command::Score(options) => score_run(options),
            Command::Generate(options) => Ok(Report {
                lines: generate::generate(options)?,
                exit: Exit::Clean,
            }),
        }
    }
}

/// The one-line usage message.
#[must_use]
pub fn usage() -> String {
    "usage: spark rs PATH... | spark text PATH... | \
     spark score RUN.jsonl [--field F] [--code-field F] [--label L] [--out OUT.jsonl] | \
     spark generate --url URL --model M --rules RULES.md --out RUN.jsonl \
     [--key-file F] [--backend NAME] [--think]"
        .to_string()
}

fn paths(arguments: &[String]) -> Outcome<Vec<PathBuf>> {
    if arguments.is_empty() {
        return Err(EvalError::Usage(usage()));
    }

    Ok(arguments.iter().map(PathBuf::from).collect())
}

impl ScoreOptions {
    fn parse(arguments: &[String]) -> Outcome<ScoreOptions> {
        let mut given = Given::default();
        let mut rest = arguments.iter();

        while let Some(argument) = rest.next() {
            given.take(argument, &mut rest)?;
        }

        given.into_options()
    }
}

/// The `score` arguments as given, before the defaults are filled in.
#[derive(Default)]
struct Given {
    input: Option<PathBuf>,
    field: Option<String>,
    code_field: Option<String>,
    label: Option<String>,
    output: Option<PathBuf>,
}

impl Given {
    /// Records `argument`, reading its value from `rest` when it is an option.
    fn take<'a>(&mut self, argument: &str, rest: &mut impl Iterator<Item = &'a String>) -> Outcome {
        let mut value = || {
            rest.next()
                .cloned()
                .ok_or_else(|| EvalError::Usage(format!("{argument} needs a value")))
        };

        match argument {
            "--field" => self.field = Some(value()?),
            "--code-field" => self.code_field = Some(value()?),
            "--label" => self.label = Some(value()?),
            "--out" => self.output = Some(PathBuf::from(value()?)),
            flag if flag.starts_with("--") => {
                let message = format!("unknown option {flag}\n{}", usage());

                return Err(EvalError::Usage(message));
            }
            path => self.input = Some(PathBuf::from(path)),
        }

        Ok(())
    }

    /// The options, with the output and the label named after the input.
    fn into_options(self) -> Outcome<ScoreOptions> {
        let Some(input) = self.input else {
            return Err(EvalError::Usage(usage()));
        };

        Ok(ScoreOptions {
            output: self
                .output
                .unwrap_or_else(|| input.with_extension("scored.jsonl")),
            label: self.label.unwrap_or_else(|| stem(&input)),
            input,
            field: self.field.unwrap_or_else(|| DEFAULT_FIELD.to_string()),
            code_field: self.code_field,
        })
    }
}

/// The `generate` options: `--url`, `--model`, `--rules` and `--out` are
/// required; `--backend` defaults to `vllm`.
fn generate_options(arguments: &[String]) -> Outcome<GenerateOptions> {
    let (mut values, think) = generate_values(arguments)?;

    let mut required = |name: &str| {
        values
            .remove(name)
            .ok_or_else(|| EvalError::Usage(format!("generate needs {name}\n{}", usage())))
    };

    Ok(GenerateOptions {
        url: required("--url")?,
        model: required("--model")?,
        rules: PathBuf::from(required("--rules")?),
        output: PathBuf::from(required("--out")?),
        key_file: values.remove("--key-file").map(PathBuf::from),
        backend: values
            .remove("--backend")
            .unwrap_or_else(|| "vllm".to_string()),
        think,
    })
}

/// The `generate` options that take a value, by name, and whether `--think`
/// was given.
fn generate_values(arguments: &[String]) -> Outcome<(BTreeMap<&str, String>, bool)> {
    let mut values = BTreeMap::new();
    let mut think = false;
    let mut rest = arguments.iter();

    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--think" => think = true,
            "--url" | "--model" | "--rules" | "--out" | "--key-file" | "--backend" => {
                let value = rest
                    .next()
                    .ok_or_else(|| EvalError::Usage(format!("{argument} needs a value")))?;
                values.insert(argument.as_str(), value.clone());
            }
            other => {
                let message = format!("unknown argument {other}\n{}", usage());

                return Err(EvalError::Usage(message));
            }
        }
    }

    Ok((values, think))
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every `.rs` file under `paths`, sorted; folders are walked, skipping
/// `target` and hidden folders.
///
/// # Errors
///
/// `EvalError::Io` when a folder can't be read.
pub fn rust_files(paths: &[PathBuf]) -> Outcome<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = paths.to_vec();

    while let Some(path) = pending.pop() {
        if path.is_dir() {
            pending.extend(entries(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }

    files.sort();

    Ok(files)
}

/// What is directly inside `folder`, without the entries that are skipped.
fn entries(folder: &Path) -> Outcome<Vec<PathBuf>> {
    let mut kept = Vec::new();

    for entry in fs::read_dir(folder).map_err(EvalError::io(folder))? {
        let entry = entry.map_err(EvalError::io(folder))?.path();

        if !is_skipped(&entry) {
            kept.push(entry);
        }
    }

    Ok(kept)
}

fn is_skipped(path: &Path) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy())
        .is_some_and(|name| SKIPPED_FOLDERS.contains(&name.as_ref()) || name.starts_with('.'))
}

fn check_rust(paths: &[PathBuf]) -> Outcome<Report> {
    let files = rust_files(paths)?;
    let mut lines = Vec::new();

    for file in &files {
        let report = rules::check(&fs::read_to_string(file).map_err(EvalError::io(file))?);

        if let Some(error) = &report.parse_error {
            lines.push(format!("{}: does not parse: {error}", file.display()));
        }

        for violation in &report.violations {
            lines.push(format!(
                "{}:{}: {}: {}",
                file.display(),
                violation.line,
                violation.rule.label(),
                violation.detail
            ));
        }
    }

    let problems = lines.len();
    lines.push(format!("{} files, {problems} problems", files.len()));

    Ok(Report {
        lines,
        exit: exit_for(problems),
    })
}

fn check_text(paths: &[PathBuf]) -> Outcome<Report> {
    let mut lines = Vec::new();
    let mut total = 0;

    for path in paths {
        let text = fs::read_to_string(path).map_err(EvalError::io(path))?;
        let report = slop::check(&text);

        for hit in &report.hits {
            let line = text[..hit.start].matches('\n').count() + 1;

            lines.push(format!(
                "{}:{line}: {}: {}",
                path.display(),
                hit.category.label(),
                hit.text
            ));
        }

        lines.push(format!(
            "{}: {} phrases, {} em dashes, {} words",
            path.display(),
            report.hits.len(),
            report.em_dashes,
            report.words
        ));
        total += report.score();
    }

    Ok(Report {
        lines,
        exit: exit_for(total),
    })
}

fn exit_for(problems: usize) -> Exit {
    if problems == 0 {
        Exit::Clean
    } else {
        Exit::Problems
    }
}

/// One answer of a run with its score.
struct Scored {
    record: Value,
    score: AnswerScore,
}

impl Scored {
    /// The answer's record with its score and the run's label added.
    fn labelled(self, options: &ScoreOptions, number: usize) -> Outcome<Value> {
        let Scored { mut record, score } = self;
        let spark =
            serde_json::to_value(&score).map_err(EvalError::json(&options.output, number))?;

        if let Value::Object(fields) = &mut record {
            fields.insert("spark".to_string(), spark);
            fields.insert(
                "spark_run".to_string(),
                Value::String(options.label.clone()),
            );
        }

        Ok(record)
    }
}

/// What a scored run prints: the table, the slop found and where it was written.
fn summary_lines(summary: &Summary, options: &ScoreOptions) -> Vec<String> {
    let categories: Vec<String> = summary
        .categories
        .iter()
        .map(|(category, count)| format!("{} {count}", category.label()))
        .collect();

    vec![
        Summary::header(),
        summary.row(&options.label),
        format!("\nslop by category: {}", categories.join(", ")),
        format!("wrote {}", options.output.display()),
    ]
}

fn score_run(options: &ScoreOptions) -> Outcome<Report> {
    let text = fs::read_to_string(&options.input).map_err(EvalError::io(&options.input))?;
    let file = fs::File::create(&options.output).map_err(EvalError::io(&options.output))?;
    let mut writer = BufWriter::new(file);
    let mut summary = Summary::default();

    for (line, number) in text
        .lines()
        .zip(1..)
        .filter(|(line, _)| !line.trim().is_empty())
    {
        let scored = score_line(line, number, options)?;
        summary.add(&scored.score);

        let record = scored.labelled(options, number)?;

        writeln!(writer, "{record}").map_err(EvalError::io(&options.output))?;
    }

    writer.flush().map_err(EvalError::io(&options.output))?;

    Ok(Report {
        lines: summary_lines(&summary, options),
        exit: Exit::Clean,
    })
}

/// Scores line `number` of the run.
fn score_line(line: &str, number: usize, options: &ScoreOptions) -> Outcome<Scored> {
    let record: Value =
        serde_json::from_str(line).map_err(EvalError::json(&options.input, number))?;
    let read = |name: &str| {
        record
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| EvalError::MissingField {
                path: options.input.clone(),
                line: number,
                field: name.to_string(),
            })
    };
    let prose = read(&options.field)?;

    let score = match &options.code_field {
        Some(code_field) => answer::score_parts(&prose, &read(code_field)?),
        None => answer::score(&prose),
    };

    Ok(Scored { record, score })
}

#[cfg(test)]
mod tests;
