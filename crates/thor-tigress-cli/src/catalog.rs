//! The models a person can choose from: the ones already on disk and the ones on the Hub,
//! each with its weights, its longest context and the shape of its KV cache.
//!
//! On disk, each engine keeps models in its own folder (settings `EDGE_ROOT`, `GGUF_ROOT`,
//! `HF_HUB_CACHE`): Edge-LLM a checkpoint folder per model, llama.cpp GGUF files in a folder
//! per model, vLLM the Hugging Face cache its container fills.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use regex::Regex;
use serde_json::Value;

use crate::{
    engine::Engine,
    hub::{Hub, HubModel, RepoFile},
    memory::KvShape,
};

/// GGUF files that help a model rather than being one: vision projectors and draft models
/// for speculative decoding. The same names `thor-tigress-serve` leaves out.
static GGUF_HELPER: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)(^|/)(mtp|dflash|dspark|draft|eagle|mmproj|shard_rewrite)[-_./]").ok()
});

/// The part suffix of a GGUF split into several files, such as `-00001-of-00003.gguf`.
static GGUF_PART: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"-\d{5}-of-\d{5}\.gguf$").ok());

/// Folders under the Edge-LLM root that hold no checkpoint.
const EDGE_HELPER_FOLDERS: [&str; 2] = ["cache", "drafters"];

/// Where a model is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// On this machine, at this path: a checkpoint folder or a GGUF file.
    Disk(PathBuf),
    /// On the Hub, not downloaded yet.
    Hub,
}

/// A model that one engine can serve.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The engine it is for.
    pub engine: Engine,
    /// Where it is.
    pub source: Source,
    /// Its Hub repository, when known.
    pub repo: Option<String>,
    /// The bytes of its weights; for a GGUF repository, its smallest variant.
    pub weights: Option<u64>,
    /// Its KV cache shape and longest context.
    pub shape: Option<KvShape>,
    /// Its architecture family, `model_type` in `config.json`, such as `gemma4`.
    pub family: Option<String>,
    /// Its `config.json`; for a GGUF repository, its base model's.
    pub config: Option<Value>,
    /// Downloads on the Hub in the last 30 days.
    pub downloads: Option<u64>,
    /// The repository's files, for a model on the Hub.
    pub files: Vec<RepoFile>,
}

impl Candidate {
    /// What the menus show: the repository when known, else the GGUF's id or the folder's
    /// name.
    pub fn name(&self) -> String {
        match (&self.repo, &self.source) {
            (Some(repo), _) => repo.to_string(),
            (None, Source::Disk(path)) => display_name(self.engine, path),
            (None, Source::Hub) => String::new(),
        }
    }

    /// Weights plus the KV cache for `context` tokens in `slots` replies.
    pub fn cost(&self, context: u64, slots: u64) -> Option<u64> {
        let cache = self.shape?.cache_bytes(context, slots);

        Some(self.weights? + cache)
    }

    /// Whether a settings value names this model: its path on disk or its repository.
    pub fn is_named_by(&self, setting: &str) -> bool {
        let path = match &self.source {
            Source::Disk(path) => path.display().to_string(),
            Source::Hub => String::new(),
        };

        !setting.is_empty()
            && [path.as_str(), self.repo.as_deref().unwrap_or_default()].contains(&setting)
    }

    /// The longest context, when known.
    pub fn max_context(&self) -> Option<u64> {
        self.shape?.max_context
    }
}

/// One quantization of a GGUF repository, which may be split into several files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GgufVariant<'files> {
    /// The variant's name: the file name without the part suffix and `.gguf`.
    pub name: String,
    /// Its files, in order.
    pub files: Vec<&'files RepoFile>,
    /// Their total size.
    pub bytes: u64,
}

/// The GGUF variants in a repository, smallest first. Helper files are left out.
pub fn gguf_variants(files: &[RepoFile]) -> Vec<GgufVariant<'_>> {
    let mut groups: BTreeMap<String, Vec<&RepoFile>> = BTreeMap::new();
    let model_files = files.iter().filter(|file| is_model_gguf(&file.path));

    for file in model_files {
        groups.entry(model_id(&file.path)).or_default().push(file);
    }

    let mut variants: Vec<GgufVariant> = groups
        .into_iter()
        .map(|(name, files)| GgufVariant {
            bytes: files.iter().map(|file| file.size).sum(),
            name,
            files,
        })
        .collect();
    variants.sort_by_key(|variant| variant.bytes);

    variants
}

/// The id llama-server lists a GGUF under: its file name without the folder, the part
/// suffix and `.gguf`, as `thor-tigress-serve` names it.
pub fn model_id(path: &str) -> String {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let without_part = GGUF_PART
        .as_ref()
        .map_or(file_name.into(), |pattern| pattern.replace(file_name, ""));

    without_part.trim_end_matches(".gguf").to_string()
}

/// The models on disk for one engine, sorted by name.
pub fn on_disk(engine: Engine, models_root: &Path) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = match engine {
        Engine::EdgeLlm => edge_checkpoints(models_root),
        Engine::LlamaCpp => gguf_files(models_root),
        Engine::Vllm => hub_cache(models_root),
    };
    found.sort_by_key(Candidate::name);

    found
}

/// A model on disk at `path`: a GGUF file, or a folder with a `config.json`.
pub fn from_disk(engine: Engine, path: &Path, repo: Option<String>) -> Candidate {
    let folder = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let weights = match engine {
        Engine::LlamaCpp => gguf_bytes(path),
        Engine::EdgeLlm | Engine::Vllm => folder_bytes(folder, ".safetensors"),
    };
    let config = read_config(folder);

    Candidate {
        engine,
        source: Source::Disk(path.to_path_buf()),
        repo,
        weights: Some(weights),
        shape: config.as_ref().map(KvShape::from_config),
        family: config.as_ref().and_then(family_of),
        config,
        downloads: None,
        files: Vec::new(),
    }
}

/// A model on the Hub, with its files, size and KV shape. A GGUF repository's shape comes
/// from its base model's `config.json`, and its longest context from the GGUF itself.
/// A file list or config the Hub does not give leaves that field unknown.
pub fn from_hub(hub: &Hub, engine: Engine, model: HubModel) -> Candidate {
    let files = hub.files(&model.id).unwrap_or_default();
    let config_repo = match engine {
        Engine::LlamaCpp => model.base_model().unwrap_or(&model.id),
        Engine::EdgeLlm | Engine::Vllm => &model.id,
    };
    let config = hub.config(config_repo).ok();
    let mut shape = config.as_ref().map(KvShape::from_config);
    let gguf_context = model.gguf.as_ref().and_then(|gguf| gguf.context_length);

    if let (Some(shape), Some(context)) = (shape.as_mut(), gguf_context) {
        shape.max_context = Some(context);
    }

    Candidate {
        engine,
        source: Source::Hub,
        weights: hub_weights(engine, &files),
        repo: Some(model.id),
        shape,
        family: config.as_ref().and_then(family_of),
        config,
        downloads: Some(model.downloads),
        files,
    }
}

/// The weights to download: the safetensors files, or a GGUF repository's smallest variant.
fn hub_weights(engine: Engine, files: &[RepoFile]) -> Option<u64> {
    let bytes = match engine {
        Engine::LlamaCpp => gguf_variants(files)
            .first()
            .map_or(0, |variant| variant.bytes),
        Engine::EdgeLlm | Engine::Vllm => files
            .iter()
            .filter(|file| file.path.ends_with(".safetensors"))
            .map(|file| file.size)
            .sum(),
    };

    (bytes > 0).then_some(bytes)
}

/// Each folder under the Edge-LLM root that holds a `config.json`.
fn edge_checkpoints(root: &Path) -> Vec<Candidate> {
    folder_entries(root)
        .into_iter()
        .filter(|path| {
            !EDGE_HELPER_FOLDERS
                .iter()
                .any(|helper| path.ends_with(helper))
        })
        .filter(|path| path.join("config.json").is_file())
        .map(|path| from_disk(Engine::EdgeLlm, &path, None))
        .collect()
}

/// Each GGUF model under the llama.cpp root, one level of folders deep. Of a split GGUF
/// only the first part is listed; its size counts every part.
fn gguf_files(root: &Path) -> Vec<Candidate> {
    folder_entries(root)
        .into_iter()
        .flat_map(|path| {
            if path.is_dir() {
                folder_entries(&path)
            } else {
                vec![path]
            }
        })
        .filter(|path| is_first_gguf(path))
        .map(|path| from_disk(Engine::LlamaCpp, &path, None))
        .collect()
}

/// Each model in the Hugging Face cache with safetensors weights: the newest snapshot of
/// every `models--ORGANIZATION--NAME` folder.
fn hub_cache(root: &Path) -> Vec<Candidate> {
    folder_entries(root)
        .into_iter()
        .filter_map(|folder| {
            let repo = repo_of_cache_folder(&folder)?;
            let snapshot = folder_entries(&folder.join("snapshots")).pop()?;

            (folder_bytes(&snapshot, ".safetensors") > 0)
                .then(|| from_disk(Engine::Vllm, &snapshot, Some(repo)))
        })
        .collect()
}

/// `ORGANIZATION/NAME` from a cache folder called `models--ORGANIZATION--NAME`.
fn repo_of_cache_folder(folder: &Path) -> Option<String> {
    let name = folder.file_name()?.to_str()?;

    Some(name.strip_prefix("models--")?.replacen("--", "/", 1))
}

/// The name of a model on disk: a GGUF's id, or the folder's name.
fn display_name(engine: Engine, path: &Path) -> String {
    let last_part = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();

    match engine {
        Engine::LlamaCpp => model_id(&last_part),
        Engine::EdgeLlm | Engine::Vllm => last_part,
    }
}

/// Whether a path is a GGUF model rather than a helper file.
fn is_model_gguf(path: &str) -> bool {
    let helper = GGUF_HELPER
        .as_ref()
        .is_some_and(|pattern| pattern.is_match(path));

    path.ends_with(".gguf") && !helper
}

/// Whether a path is a GGUF model file that is not a later part of a split one.
fn is_first_gguf(path: &Path) -> bool {
    let name = path.to_string_lossy();
    let first_or_only_part = !name.contains("-of-") || name.contains("-00001-of-");

    is_model_gguf(&name) && first_or_only_part
}

/// The bytes of a GGUF, adding every part of a split one.
fn gguf_bytes(path: &Path) -> u64 {
    let id = model_id(&path.to_string_lossy());
    let folder = path.parent().unwrap_or(path);

    folder_entries(folder)
        .iter()
        .filter(|part| {
            model_id(&part.to_string_lossy()) == id && part.to_string_lossy().ends_with(".gguf")
        })
        .filter_map(|part| fs::metadata(part).ok())
        .map(|metadata| metadata.len())
        .sum()
}

/// The bytes of the files in a folder whose names end with `suffix`. Symbolic links, as in
/// the Hugging Face cache, count as the files they point to.
fn folder_bytes(folder: &Path, suffix: &str) -> u64 {
    folder_entries(folder)
        .iter()
        .filter(|path| path.to_string_lossy().ends_with(suffix))
        .filter_map(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum()
}

/// The architecture family a `config.json` names.
fn family_of(config: &Value) -> Option<String> {
    config
        .get("model_type")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The `config.json` in a folder, when there is a readable one.
fn read_config(folder: &Path) -> Option<Value> {
    let text = fs::read_to_string(folder.join("config.json")).ok()?;

    serde_json::from_str(&text).ok()
}

/// The entries of a folder, sorted; empty when it cannot be read.
fn folder_entries(folder: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    entries
}
