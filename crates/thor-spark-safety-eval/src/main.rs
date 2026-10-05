//! `spark`: the Stage 0 command line.
//!
//! ```text
//! spark rs    PATH...    check .rs files (directories are walked) against the five rules
//! spark text  PATH...    find slop in text or markdown files
//! spark score RUN.jsonl [--field F] [--code-field F] [--label L] [--out OUT.jsonl]
//! ```
//!
//! `score` reads one answer per JSONL line from `--field` (default `text`). With
//! `--code-field`, the code is read from that field and the prose from
//! `--field`. Each line is written back with a `spark` object added, to
//! `--out` (default `RUN.scored.jsonl`) with the label in `spark_run`, and a
//! summary row is printed.
//! `rs` exits with status 1 when any rule is broken, so it can gate a build.

use std::{
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use serde_json::Value;
use thor_spark_safety_eval::{
    answer,
    error::EvalError,
    report::Summary,
    rules,
    slop,
};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match arguments.split_first() {
        Some((command, rest)) if command == "rs" => check_rust(rest),
        Some((command, rest)) if command == "text" => check_text(rest),
        Some((command, rest)) if command == "score" => score_run(rest),
        _ => Err(EvalError::Usage(usage())),
    };
    match outcome {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("spark: {error}");
            ExitCode::from(2)
        }
    }
}

fn usage() -> String {
    "usage: spark rs PATH... | spark text PATH... | \
     spark score RUN.jsonl [--field F] [--code-field F] [--label L] [--out OUT.jsonl]"
        .to_string()
}

fn check_rust(paths: &[String]) -> Result<bool, EvalError> {
    let files = rust_files(paths)?;
    let mut broken = 0usize;
    for file in &files {
        let source = fs::read_to_string(file).map_err(EvalError::io(file))?;
        let report = rules::check(&source);
        if let Some(error) = &report.parse_error {
            println!("{}: does not parse: {error}", file.display());
            broken += 1;
        }
        for violation in &report.violations {
            println!(
                "{}:{}: {}: {}",
                file.display(),
                violation.line,
                violation.rule.label(),
                violation.detail
            );
        }
        broken += report.violations.len();
    }
    println!("{} files, {broken} problems", files.len());
    Ok(broken == 0)
}

fn rust_files(paths: &[String]) -> Result<Vec<PathBuf>, EvalError> {
    if paths.is_empty() {
        return Err(EvalError::Usage(usage()));
    }
    let mut files = Vec::new();
    let mut pending: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            let entries = fs::read_dir(&path).map_err(EvalError::io(&path))?;
            for entry in entries {
                let entry = entry.map_err(EvalError::io(&path))?.path();
                let skipped = entry
                    .file_name()
                    .is_some_and(|name| name == "target" || name.to_string_lossy().starts_with('.'));
                if !skipped {
                    pending.push(entry);
                }
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn check_text(paths: &[String]) -> Result<bool, EvalError> {
    if paths.is_empty() {
        return Err(EvalError::Usage(usage()));
    }
    let mut total = 0usize;
    for path in paths {
        let text = fs::read_to_string(path).map_err(EvalError::io(path))?;
        let report = slop::check(&text);
        for hit in &report.hits {
            let line = text[..hit.start].matches('\n').count() + 1;
            println!("{path}:{line}: {}: {}", hit.category.label(), hit.text);
        }
        println!(
            "{path}: {} phrases, {} em dashes, {} words",
            report.hits.len(),
            report.em_dashes,
            report.words
        );
        total += report.score();
    }
    Ok(total == 0)
}

/// The options of `spark score`.
struct ScoreOptions {
    input: PathBuf,
    field: String,
    code_field: Option<String>,
    label: String,
    output: PathBuf,
}

fn score_options(arguments: &[String]) -> Result<ScoreOptions, EvalError> {
    let mut input = None;
    let mut field = "text".to_string();
    let mut code_field = None;
    let mut label = None;
    let mut output = None;
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        let mut value = || {
            rest.next()
                .cloned()
                .ok_or_else(|| EvalError::Usage(format!("{argument} needs a value")))
        };
        match argument.as_str() {
            "--field" => field = value()?,
            "--code-field" => code_field = Some(value()?),
            "--label" => label = Some(value()?),
            "--out" => output = Some(PathBuf::from(value()?)),
            flag if flag.starts_with("--") => {
                return Err(EvalError::Usage(format!("unknown option {flag}\n{}", usage())));
            }
            path => input = Some(PathBuf::from(path)),
        }
    }
    let input = input.ok_or_else(|| EvalError::Usage(usage()))?;
    let output = output.unwrap_or_else(|| input.with_extension("scored.jsonl"));
    let label = label.unwrap_or_else(|| stem(&input));
    Ok(ScoreOptions {
        input,
        field,
        code_field,
        label,
        output,
    })
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn score_run(arguments: &[String]) -> Result<bool, EvalError> {
    let options = score_options(arguments)?;
    let text = fs::read_to_string(&options.input).map_err(EvalError::io(&options.input))?;
    let file = fs::File::create(&options.output).map_err(EvalError::io(&options.output))?;
    let mut writer = BufWriter::new(file);
    let mut summary = Summary::default();
    let lines = text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty());
    for (index, line) in lines {
        let mut record: Value = serde_json::from_str(line).map_err(|source| EvalError::Json {
            path: options.input.clone(),
            line: index + 1,
            source,
        })?;
        let read = |name: &str| {
            record
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| EvalError::MissingField {
                    path: options.input.clone(),
                    line: index + 1,
                    field: name.to_string(),
                })
        };
        let prose = read(&options.field)?;
        let score = match &options.code_field {
            Some(code_field) => answer::score_parts(&prose, &read(code_field)?),
            None => answer::score(&prose),
        };
        summary.add(&score);
        let spark = serde_json::to_value(&score).map_err(|source| EvalError::Json {
            path: options.output.clone(),
            line: index + 1,
            source,
        })?;
        if let Value::Object(fields) = &mut record {
            fields.insert("spark".to_string(), spark);
            fields.insert("spark_run".to_string(), Value::String(options.label.clone()));
        }
        writeln!(writer, "{record}").map_err(EvalError::io(&options.output))?;
    }
    writer.flush().map_err(EvalError::io(&options.output))?;
    println!("{}", Summary::header());
    println!("{}", summary.row(&options.label));
    let categories: Vec<String> = summary
        .categories
        .iter()
        .map(|(category, count)| format!("{} {count}", category.label()))
        .collect();
    println!("\nslop by category: {}", categories.join(", "));
    println!("wrote {}", options.output.display());
    Ok(true)
}
