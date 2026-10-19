//! `spark generate`: collects a run for `spark score` from an
//! OpenAI-compatible server.
//!
//! Each run is the conversation the Stage 0 baselines were collected with
//! (`edgechat/eval/run.py`): a config-file parser, then a follow-up asking for
//! `[section]` support and the full updated code. The system prompt, the
//! sampling settings and the plan of temperatures and runs are the same, so
//! a new model's scores sit next to the baselines' as like for like.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use ureq::{Agent, http::StatusCode};

use crate::error::{EvalError, Outcome};

/// The two user turns of every run.
pub const TURNS: [&str; 2] = [
    "Write a Rust function that parses a config file of `key = value` lines (skip blank lines and lines \
     starting with #) into a HashMap<String, String>, with a unit test. Explain your design in detail.",
    "Now add support for `[section]` headers: keys under a section become `section.key`. \
     Show the full updated code.",
];

/// The system prompt before the rules are appended: local-copilot-codebuddy's
/// default, which the baselines were collected with.
const SYSTEM_PREFIX: &str = "You are Qwen, created by Alibaba Cloud. You are a helpful assistant. \
     Match the depth of your answer to the request: answer simple questions briefly, but when asked \
     to explain or go into detail, write a long, thorough, well-structured answer with headings, \
     examples and code, like a chapter of a good technical book.\n\nThe user's coding rules. Follow \
     them in every answer, for the whole conversation:\n\n";

/// The temperatures and how many runs each gets.
pub const PLAN: [(f64, u32); 3] = [(0.0, 2), (0.2, 5), (0.7, 5)];

/// Qwen's `generation_config.json` sampling, used for every model.
const TOP_P: f64 = 0.8;

/// See [`TOP_P`].
const TOP_K: u32 = 20;

/// See [`TOP_P`].
const REPETITION_PENALTY: f64 = 1.1;

/// The most tokens one answer may take.
const MAX_TOKENS: u32 = 8192;

/// A run's seed is this plus the run number, as in the baselines.
const SEED_BASE: u32 = 1000;

/// How long one answer may take.
const TIMEOUT: Duration = Duration::from_secs(1800);

/// The options of `spark generate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateOptions {
    /// The server, such as `http://127.0.0.1:8000`; `/v1/chat/completions`
    /// is appended.
    pub url: String,
    /// The model name the server expects.
    pub model: String,
    /// The coding rules appended to the system prompt.
    pub rules: PathBuf,
    /// A file holding a bearer key, when the server wants one.
    pub key_file: Option<PathBuf>,
    /// Whether the model thinks before it answers.
    pub think: bool,
    /// The `backend` written into every line.
    pub backend: String,
    /// The JSONL file answers are appended to.
    pub output: PathBuf,
}

/// One answer and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// The visible answer.
    pub text: String,
    /// The thinking, when the server returned it separately.
    pub reasoning: Option<String>,
    /// Generated tokens, thinking included.
    pub tokens: Option<u64>,
}

/// Collects every run of [`PLAN`] and returns one line per run.
///
/// # Errors
///
/// `EvalError::Io` when the rules, the key or the output can't be read or
/// written, and `EvalError::Server` when the server fails or answers with
/// something that is not a chat completion.
pub fn generate(options: &GenerateOptions) -> Outcome<Vec<String>> {
    let rules = fs::read_to_string(&options.rules).map_err(EvalError::io(&options.rules))?;
    let key = options.key_file.as_deref().map(read_key).transpose()?;
    let system = format!("{SYSTEM_PREFIX}{}", rules.trim());

    let runs = PLAN
        .iter()
        .flat_map(|&(temperature, count)| (0..count).map(move |run| (temperature, run)));

    runs.map(|(temperature, run)| {
        let started = Instant::now();
        collect_run(options, key.as_deref(), &system, temperature, run)?;

        Ok(format!(
            "{} T={temperature} run {run}: {:.0}s",
            options.backend,
            started.elapsed().as_secs_f64()
        ))
    })
    .collect()
}

/// Asks both turns of one run and appends each answer to the output.
fn collect_run(
    options: &GenerateOptions,
    key: Option<&str>,
    system: &str,
    temperature: f64,
    run: u32,
) -> Outcome {
    let mut messages = vec![json!({ "role": "system", "content": system })];

    for (turn, question) in (1..).zip(TURNS) {
        messages.push(json!({ "role": "user", "content": question }));

        let body = request_body(&options.model, &messages, temperature, run, options.think);
        let started = Instant::now();
        let reply = post(&options.url, key, &body.to_string())?;
        let seconds = started.elapsed().as_secs_f64();
        let answer = answer_from_reply(&reply)?;

        messages.push(json!({ "role": "assistant", "content": answer.text }));
        append(
            &options.output,
            &record(options, temperature, run, turn, &answer, seconds),
        )?;
    }

    Ok(())
}

/// The chat request for one turn.
#[must_use]
pub fn request_body(
    model: &str,
    messages: &[Value],
    temperature: f64,
    run: u32,
    think: bool,
) -> Value {
    json!({
        "model": model,
        "messages": messages,
        "max_tokens": MAX_TOKENS,
        "temperature": temperature,
        "top_p": TOP_P,
        "top_k": TOP_K,
        "repetition_penalty": REPETITION_PENALTY,
        "seed": SEED_BASE + run,
        "chat_template_kwargs": { "enable_thinking": think },
    })
}

/// The answer in a chat completion reply.
///
/// # Errors
///
/// `EvalError::Server` when `reply` is not JSON or has no message content.
pub fn answer_from_reply(reply: &str) -> Outcome<Answer> {
    let parsed: Value = serde_json::from_str(reply)
        .map_err(|error| EvalError::Server(format!("reply is not JSON: {error}")))?;
    let message = parsed
        .pointer("/choices/0/message")
        .ok_or_else(|| EvalError::Server(format!("reply has no message: {reply}")))?;
    let reasoning = ["reasoning_content", "reasoning"]
        .iter()
        .find_map(|field| message.get(field).and_then(Value::as_str))
        .map(str::to_string);

    Ok(Answer {
        text: message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        reasoning,
        tokens: parsed
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64),
    })
}

/// The JSONL line for one answer, in the baselines' shape plus the model,
/// thinking, the seconds it took and its thinking text.
fn record(
    options: &GenerateOptions,
    temperature: f64,
    run: u32,
    turn: u32,
    answer: &Answer,
    seconds: f64,
) -> Value {
    let tok_per_s = answer
        .tokens
        .filter(|_| seconds > 0.0)
        .map(|tokens| (tokens as f64 / seconds * 100.0).round() / 100.0);

    json!({
        "backend": options.backend,
        "model": options.model,
        "think": options.think,
        "temperature": temperature,
        "run": run,
        "turn": turn,
        "text": answer.text,
        "reasoning": answer.reasoning,
        "tokens": answer.tokens,
        "seconds": (seconds * 10.0).round() / 10.0,
        "tok_per_s": tok_per_s,
    })
}

fn append(path: &Path, line: &Value) -> Outcome {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(EvalError::io(path))?;

    writeln!(file, "{line}").map_err(EvalError::io(path))
}

fn read_key(path: &Path) -> Outcome<String> {
    Ok(fs::read_to_string(path)
        .map_err(EvalError::io(path))?
        .trim()
        .to_string())
}

fn post(url: &str, key: Option<&str>, body: &str) -> Outcome<String> {
    let server = |error: ureq::Error| EvalError::Server(format!("{url}: {error}"));
    let agent: Agent = Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let request = agent
        .post(format!("{}/v1/chat/completions", url.trim_end_matches('/')))
        .header("Content-Type", "application/json");

    let request = match key {
        Some(key) => request.header("Authorization", format!("Bearer {key}")),
        None => request,
    };

    let mut response = request.send(body).map_err(server)?;
    let text = response.body_mut().read_to_string().map_err(server)?;

    match response.status() {
        StatusCode::OK => Ok(text),
        status => Err(EvalError::Server(format!("{status} {}", text.trim()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_with_the_baseline_sampling_and_the_thinking_switch() {
        let messages = [json!({ "role": "user", "content": "hi" })];

        let body = request_body("qwen", &messages, 0.2, 3, false);

        assert_eq!(body["model"], "qwen");
        assert_eq!(body["top_k"], 20);
        assert_eq!(body["seed"], 1003);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn reads_the_answer_its_thinking_and_its_tokens() -> Outcome {
        let reply = r#"{"choices":[{"message":{"content":"done","reasoning_content":"plan"}}],
                        "usage":{"completion_tokens":42}}"#;

        let answer = answer_from_reply(reply)?;

        assert_eq!(answer.text, "done");
        assert_eq!(answer.reasoning.as_deref(), Some("plan"));
        assert_eq!(answer.tokens, Some(42));

        Ok(())
    }

    #[test]
    fn refuses_a_reply_without_a_message() {
        assert!(matches!(answer_from_reply("{}"), Err(EvalError::Server(_))));
        assert!(matches!(
            answer_from_reply("nope"),
            Err(EvalError::Server(_))
        ));
    }

    #[test]
    fn plans_the_same_twelve_runs_as_the_baselines() {
        let runs: u32 = PLAN.iter().map(|&(_, count)| count).sum();

        assert_eq!(runs, 12);
    }
}
