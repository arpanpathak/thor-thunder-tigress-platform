//! The `thor` command line.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use crate::{
    engine::Engine,
    error::{Outcome, ThorError},
};

/// The help text.
pub const USAGE: &str = "\
usage: thor COMMAND [OPTIONS]

  thor engines              what each engine is for, and whether it is installed here
  thor serve                choose an engine, a model and a context, then serve it
  thor list                 the models on disk and the services running, with memory
  thor stop [NAME]          stop a service, such as thor-edge-llm
  thor logs [NAME]          follow a service's log

serve options, for scripts:
  --engine edge-llm|llama.cpp|vllm
  --model NAME|FOLDER       a Hugging Face repository, or a model already on disk
  --file VARIANT            for a GGUF repository, the variant, such as Q4_K_M
  --context max|TOKENS      the context window; max is the model's own maximum
  --reply-tokens TOKENS     tokens kept for the reply when the prompt is capped (8192)
  --limit COUNT             Hugging Face results per search (12)
  --wait SECONDS            how long to wait for the server to answer (3600)
  --no-docs                 do not fetch the engine's recipes and documentation
  --yes                     do not ask for confirmation

every command:
  --settings FILE           the settings file (~/.config/thor-chat/env)";

/// The flags that take a value.
const VALUE_FLAGS: [&str; 8] = [
    "engine",
    "model",
    "file",
    "context",
    "reply-tokens",
    "limit",
    "wait",
    "settings",
];

/// How large a context to serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextChoice {
    /// The model's own maximum, from its `config.json`.
    Max,
    /// This many tokens.
    Tokens(u64),
}

/// What `thor serve` was told on the command line; anything left out is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOptions {
    /// The engine.
    pub engine: Option<Engine>,
    /// A Hugging Face repository or a path on disk.
    pub model: Option<String>,
    /// For a GGUF repository, the variant's name.
    pub file: Option<String>,
    /// The context window.
    pub context: Option<ContextChoice>,
    /// Tokens kept for the reply when the engine caps the prompt.
    pub reply_tokens: u64,
    /// Results per Hub search.
    pub limit: usize,
    /// How long to wait for the server to answer.
    pub wait: Duration,
    /// Fetch and print the engine's recipes and documentation for the model.
    pub docs: bool,
    /// Skip the confirmation.
    pub yes: bool,
}

/// One command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `thor engines`.
    Engines,
    /// `thor serve`.
    Serve(ServeOptions),
    /// `thor list`.
    List,
    /// `thor stop [NAME]`.
    Stop {
        /// The service, or `None` to choose from a menu.
        name: Option<String>,
        /// Skip the confirmation.
        yes: bool,
    },
    /// `thor logs [NAME]`.
    Logs {
        /// The service, or `None` to choose from a menu.
        name: Option<String>,
    },
    /// `thor`, `thor help` or `--help`.
    Help,
}

/// The parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    /// The command.
    pub command: Command,
    /// `--settings`, when given.
    pub settings: Option<PathBuf>,
}

/// Reads the command line, without the program's name.
pub fn parse(arguments: &[String]) -> Outcome<Cli> {
    let yes = arguments
        .iter()
        .any(|argument| is_one_of(argument, &["--yes", "-y"]));
    let help = arguments
        .iter()
        .any(|argument| is_one_of(argument, &["--help", "-h"]));
    let docs = !arguments.iter().any(|argument| argument == "--no-docs");
    let (words, values) = words_and_flags(arguments)?;
    let settings = values.get("settings").map(PathBuf::from);

    if help {
        return Ok(Cli {
            command: Command::Help,
            settings,
        });
    }

    let mut words = words.into_iter();
    let command = match words.next().as_deref() {
        None | Some("help") => Command::Help,
        Some("engines") => Command::Engines,
        Some("serve") => Command::Serve(serve_options(&values, yes, docs)?),
        Some("list") => Command::List,
        Some("stop") => Command::Stop {
            name: words.next(),
            yes,
        },
        Some("logs") => Command::Logs { name: words.next() },
        Some(other) => return Err(ThorError::Usage(format!("no command {other:?}"))),
    };

    Ok(Cli { command, settings })
}

/// Whether an argument is one of the given spellings.
fn is_one_of(argument: &str, spellings: &[&str]) -> bool {
    spellings.contains(&argument)
}

/// Splits the arguments into plain words and `--flag value` pairs. `--yes`, `--help` and
/// `--no-docs` are read by [`parse`] and skipped here.
fn words_and_flags(arguments: &[String]) -> Outcome<(Vec<String>, BTreeMap<String, String>)> {
    let mut words = Vec::new();
    let mut values = BTreeMap::new();
    let mut rest = arguments
        .iter()
        .filter(|argument| !is_one_of(argument, &["--yes", "-y", "--help", "-h", "--no-docs"]));

    while let Some(argument) = rest.next() {
        match argument.strip_prefix("--") {
            None => words.push(argument.to_string()),
            Some(flag) => {
                values.insert(flag.to_string(), flag_value(flag, rest.next())?);
            }
        }
    }

    Ok((words, values))
}

/// The value after a flag, when the flag is one `thor` knows and a value follows it.
fn flag_value(flag: &str, next: Option<&String>) -> Outcome<String> {
    if !VALUE_FLAGS.contains(&flag) {
        return Err(ThorError::Usage(format!("no option --{flag}")));
    }

    next.map(String::to_string)
        .ok_or_else(|| ThorError::Usage(format!("--{flag} needs a value")))
}

/// The options of `thor serve`.
fn serve_options(
    values: &BTreeMap<String, String>,
    yes: bool,
    docs: bool,
) -> Outcome<ServeOptions> {
    let engine = values
        .get("engine")
        .map(|name| Engine::from_flag(name))
        .transpose()?;
    let context = values
        .get("context")
        .map(|text| parse_context(text))
        .transpose()?;

    Ok(ServeOptions {
        engine,
        model: values.get("model").map(String::to_string),
        file: values.get("file").map(String::to_string),
        context,
        reply_tokens: number_flag(values, "reply-tokens", 8192)?,
        limit: number_flag(values, "limit", 12)? as usize,
        wait: Duration::from_secs(number_flag(values, "wait", 3600)?),
        docs,
        yes,
    })
}

/// `max` or a number of tokens.
pub fn parse_context(text: &str) -> Outcome<ContextChoice> {
    if text.eq_ignore_ascii_case("max") {
        return Ok(ContextChoice::Max);
    }

    text.replace([',', '_'], "")
        .parse()
        .map(ContextChoice::Tokens)
        .map_err(|error| {
            ThorError::Usage(format!(
                "--context {text:?}: {error}; use max or a number of tokens"
            ))
        })
}

/// A whole-number flag, or its default when it is not given.
fn number_flag(values: &BTreeMap<String, String>, name: &str, default: u64) -> Outcome<u64> {
    let Some(text) = values.get(name) else {
        return Ok(default);
    };

    text.parse()
        .map_err(|error| ThorError::Usage(format!("--{name} {text:?}: {error}")))
}
