//! What `thor-tigress serve` is about to do, worked out before anything changes, so it can be
//! shown and confirmed.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use regex::Regex;

use crate::{
    catalog::{self, Candidate, Source},
    edge,
    engine::Engine,
    machine::{Machine, tilde},
    memory::{Budget, gigabytes, tokens},
    settings::Settings,
};

/// The service that serves the chat page; it learns the engines' model names when it starts.
pub const AGENT_SERVICE: &str = "thor-tigress-agent";

/// vLLM's context option, written either as `--max-model-len N` or `--max-model-len=N`.
static MAX_MODEL_LEN: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"--max-model-len(?:=|\s+)\S+\s*").ok());

/// The changes `thor-tigress serve` makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The engine.
    pub engine: Engine,
    /// The engine's port, from the settings.
    pub port: String,
    /// The settings file written.
    pub settings_path: PathBuf,
    /// The home folder, for printing paths with `~`.
    pub home: PathBuf,
    /// What the settings call the model: a path on disk, or a repository for vLLM.
    pub target: String,
    /// Where the files go, when there is something to download.
    pub download_folder: Option<PathBuf>,
    /// The settings to write, in order.
    pub changes: Vec<(&'static str, String)>,
    /// The id the server lists the model under on `/v1/models`.
    pub served_name: String,
    /// The context window.
    pub context: u64,
    /// Whether the chat page's service must restart to offer the model under its new name.
    pub restart_agent: bool,
    /// A note on an Edge-LLM build with a draft model larger than any known to build here.
    pub build_note: Option<String>,
}

impl Plan {
    /// Works out the plan for a model and context.
    pub fn new(machine: &Machine, candidate: &Candidate, context: u64, reply_tokens: u64) -> Plan {
        let root = machine.models_root(candidate.engine);
        let (target, download_folder) = placement(candidate, &root);
        let served_name = served_name(candidate.engine, &target);
        let changes = settings_changes(
            &machine.settings,
            candidate.engine,
            &target,
            &served_name,
            context,
            reply_tokens,
        );
        let model_changes = machine.settings.value(candidate.engine.model_key()) != target;
        let build_note =
            edge_build_note(machine, candidate, context, reply_tokens).map(|note| note.text);

        Plan {
            engine: candidate.engine,
            port: machine.settings.value(candidate.engine.port_key()),
            settings_path: machine.settings.path.to_path_buf(),
            home: machine.home.to_path_buf(),
            restart_agent: model_changes && candidate.engine != Engine::LlamaCpp,
            build_note,
            target,
            download_folder,
            changes,
            served_name,
            context,
        }
    }

    /// The services restarted, in order.
    pub fn services(&self) -> Vec<&'static str> {
        let agent = self.restart_agent.then_some(AGENT_SERVICE);

        std::iter::once(self.engine.service())
            .chain(agent)
            .collect()
    }

    /// What will be downloaded, and where to.
    pub fn download_text(&self, candidate: &Candidate) -> String {
        match (&self.download_folder, candidate.files.is_empty()) {
            (Some(folder), false) => format!(
                "{} files, {} into {} (files already there are kept)",
                candidate.files.len(),
                gigabytes(candidate.files.iter().map(|file| file.size).sum()),
                tilde(folder, &self.home)
            ),
            (None, _) | (Some(_), true) => "nothing".to_string(),
        }
    }

    /// The settings to write, one indented `KEY=value` per line, with `~` for the home folder.
    fn settings_lines(&self) -> String {
        let lines: Vec<String> = self
            .changes
            .iter()
            .map(|(key, value)| {
                format!(
                    "               {key}={}",
                    tilde(Path::new(value), &self.home)
                )
            })
            .collect();

        lines.join("\n")
    }

    /// Prints the plan.
    pub fn print(&self, candidate: &Candidate, budget: &Budget, slots: u64) {
        let cost = candidate
            .cost(self.context, slots)
            .map_or_else(|| "unknown".to_string(), gigabytes);
        println!(
            "\nPlan\n  engine     {} ({} on port {})\n  model      {}\n  download   {}\n  \
             context    {} tokens, {slots} at once\n  memory     about {cost}; {} can be used\n  \
             settings   {} (backed up first)\n{}\n  restart    {}\n{}",
            self.engine.title(),
            self.engine.service(),
            self.port,
            candidate.name(),
            self.download_text(candidate),
            tokens(self.context),
            gigabytes(budget.usable()),
            tilde(&self.settings_path, &self.home),
            self.settings_lines(),
            self.services().join(", then "),
            self.build_note
                .as_ref()
                .map(|note| format!("\n{note}\n"))
                .unwrap_or_default(),
        );
    }
}

/// The drafter build note for an Edge-LLM model that has a draft model on disk, compared
/// with the builds recorded in `EDGE_WORKSPACE_BUILT_GIB` and `EDGE_WORKSPACE_FAILED_GIB`.
pub fn edge_build_note(
    machine: &Machine,
    candidate: &Candidate,
    context: u64,
    reply_tokens: u64,
) -> Option<edge::BuildNote> {
    let root = machine.models_root(Engine::EdgeLlm);
    let folder = candidate
        .name()
        .rsplit('/')
        .next()
        .map(str::to_string)
        .unwrap_or_default();
    let has_drafter = edge::drafter_folder(&root, &folder).is_dir();
    let recorded = edge::RecordedBuilds {
        built: machine
            .settings
            .number("EDGE_WORKSPACE_BUILT_GIB")
            .unwrap_or(0)
            * edge::GIBIBYTE,
        failed: machine
            .settings
            .number("EDGE_WORKSPACE_FAILED_GIB")
            .unwrap_or(0)
            * edge::GIBIBYTE,
    };
    let vocab_size = candidate.shape.and_then(|shape| shape.vocab_size);
    let prompt = context.saturating_sub(reply_tokens);

    (candidate.engine == Engine::EdgeLlm && has_drafter)
        .then(|| edge::drafter_build_note(vocab_size, context, prompt, recorded))
        .flatten()
}

/// What the settings will call the model, and the folder to download into when the model
/// is not on disk yet. An Edge-LLM checkpoint goes to `EDGE_ROOT/NAME`; a GGUF to
/// `GGUF_ROOT/NAME` without `-GGUF`; vLLM needs only the repository's name.
pub fn placement(candidate: &Candidate, root: &Path) -> (String, Option<PathBuf>) {
    let repo = candidate.repo.as_deref().unwrap_or_default();
    let repo_name = repo.rsplit('/').next().unwrap_or(repo);
    let first_file = candidate
        .files
        .first()
        .map(|file| file.path.as_str())
        .unwrap_or_default();

    match (&candidate.source, candidate.engine) {
        (_, Engine::Vllm) => (repo.to_string(), None),
        (Source::Disk(path), Engine::EdgeLlm | Engine::LlamaCpp) => {
            (path.display().to_string(), None)
        }
        (Source::Hub, Engine::EdgeLlm) => {
            let folder = root.join(repo_name);
            (folder.display().to_string(), Some(folder))
        }
        (Source::Hub, Engine::LlamaCpp) => {
            let folder = root.join(without_gguf_suffix(repo_name));
            (folder.join(first_file).display().to_string(), Some(folder))
        }
    }
}

/// A repository name without a trailing `-GGUF`, in any case.
fn without_gguf_suffix(name: &str) -> &str {
    let cut = name.len().saturating_sub("-gguf".len());
    let has_suffix = name.to_lowercase().ends_with("-gguf");

    if has_suffix { &name[..cut] } else { name }
}

/// The id the engine's server lists the model under: the checkpoint folder's name for
/// Edge-LLM (`--served-model-name`), the GGUF's id for llama.cpp, the repository's last
/// part for vLLM (`VLLM_NAME`).
pub fn served_name(engine: Engine, target: &str) -> String {
    let last_part = target.rsplit('/').next().unwrap_or(target);

    match engine {
        Engine::LlamaCpp => catalog::model_id(last_part),
        Engine::EdgeLlm | Engine::Vllm => last_part.to_string(),
    }
}

/// The settings each engine reads. Edge-LLM caps the longest prompt at `EDGE_PROMPT`,
/// which keeps `reply_tokens` of the window for the answer. vLLM takes its window as
/// `--max-model-len` inside `VLLM_ARGS`; the other options there are kept.
pub fn settings_changes(
    settings: &Settings,
    engine: Engine,
    target: &str,
    served_name: &str,
    context: u64,
    reply_tokens: u64,
) -> Vec<(&'static str, String)> {
    match engine {
        Engine::EdgeLlm => vec![
            ("EDGE_MODEL", target.to_string()),
            ("EDGE_CONTEXT", context.to_string()),
            (
                "EDGE_PROMPT",
                context.saturating_sub(reply_tokens).to_string(),
            ),
        ],
        Engine::LlamaCpp => vec![
            ("MODEL", target.to_string()),
            ("CONTEXT", context.to_string()),
        ],
        Engine::Vllm => vec![
            ("VLLM_MODEL", target.to_string()),
            ("VLLM_NAME", served_name.to_string()),
            (
                "VLLM_ARGS",
                with_max_model_len(&settings.value("VLLM_ARGS"), context),
            ),
        ],
    }
}

/// vLLM's options with `--max-model-len` set to `context`, replacing an earlier one in
/// either the `--max-model-len N` or the `--max-model-len=N` form. The rest of the options
/// keep their exact text, spaces inside quoted JSON included.
///
/// The new `--max-model-len N` always goes last. `thor-tigress-serve` strips every quote
/// character from both ends of a settings value, so a value ending in a quoted JSON option
/// such as `--speculative-config '{...}'` loses its closing quote and vLLM's command line
/// no longer parses. Ending the value with a number keeps such an option away from the end.
pub fn with_max_model_len(arguments: &str, context: u64) -> String {
    let without = MAX_MODEL_LEN
        .as_ref()
        .map_or(Cow::Borrowed(arguments), |pattern| {
            pattern.replace_all(arguments, "")
        });
    let setting = format!("--max-model-len {context}");

    format!("{} {setting}", without.trim())
        .trim_start()
        .to_string()
}
