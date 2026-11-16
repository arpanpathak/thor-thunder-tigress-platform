//! The three engines that serve models on the Thor, and what each one needs.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use crate::{
    error::{Outcome, ThorError},
    settings::Settings,
};

/// A program that serves models over an OpenAI-compatible API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// NVIDIA TensorRT Edge-LLM: builds a TensorRT engine on the Thor from a quantized
    /// checkpoint.
    EdgeLlm,
    /// llama.cpp's `llama-server`: runs GGUF files.
    LlamaCpp,
    /// vLLM in a Docker container: runs Hugging Face checkpoints.
    Vllm,
}

/// Whether an engine can run on this machine, and what was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// Found; the text says where.
    Yes(String),
    /// Not found; the text says what is missing.
    No(String),
}

impl Engine {
    /// Every engine, in the order format detection tries them: GGUF first, because a GGUF
    /// repository can also carry a `config.json`, then quantized checkpoints, then plain ones.
    pub const ALL: [Engine; 3] = [Engine::LlamaCpp, Engine::EdgeLlm, Engine::Vllm];

    /// The engine a `--engine` value names.
    pub fn from_flag(name: &str) -> Outcome<Engine> {
        Engine::ALL
            .into_iter()
            .find(|engine| engine.flag() == name.to_ascii_lowercase())
            .ok_or_else(|| {
                ThorError::Usage(format!(
                    "no engine {name:?}; use edge-llm, llama.cpp or vllm"
                ))
            })
    }

    /// The name used on the command line.
    pub fn flag(self) -> &'static str {
        match self {
            Engine::EdgeLlm => "edge-llm",
            Engine::LlamaCpp => "llama.cpp",
            Engine::Vllm => "vllm",
        }
    }

    /// The engines in the order the menu shows them.
    pub const MENU: [Engine; 3] = [Engine::EdgeLlm, Engine::LlamaCpp, Engine::Vllm];

    /// Whether this engine serves models in `format`'s format. vLLM also serves the
    /// quantized checkpoints made for Edge-LLM.
    pub fn can_serve(self, format: Engine) -> bool {
        match (self, format) {
            (Engine::Vllm, Engine::EdgeLlm) => true,
            (engine, format) => engine == format,
        }
    }

    /// The engine's product name.
    pub fn title(self) -> &'static str {
        match self {
            Engine::EdgeLlm => "TensorRT Edge-LLM",
            Engine::LlamaCpp => "llama.cpp",
            Engine::Vllm => "vLLM",
        }
    }

    /// One line for the engine menu: when to choose it.
    pub fn summary(self) -> &'static str {
        match self {
            Engine::EdgeLlm => {
                "NVFP4; fastest per token; prompt length fixed when the engine is built"
            }
            Engine::LlamaCpp => "GGUF; the widest choice of models and sizes; starts in seconds",
            Engine::Vllm => "Hugging Face checkpoints; long prompts, prefix caching, MTP drafters",
        }
    }

    /// One paragraph saying what the engine is and when to choose it.
    pub fn explanation(self) -> &'static str {
        match self {
            Engine::EdgeLlm => {
                "NVIDIA's engine for the Thor. It serves checkpoints quantized with NVIDIA \
                 Model Optimizer (NVFP4, FP8) and is the fastest of the three per token. It builds \
                 a TensorRT engine on first start, which takes minutes; later starts reuse the \
                 engine cache. The longest prompt is fixed when the engine is built, and a longer \
                 prompt is not accepted. With a draft model and a large vocabulary, a long build \
                 length can make the build fail; thor serve warns before such a build. Choose it \
                 for short and medium prompts."
            }
            Engine::LlamaCpp => {
                "llama-server runs GGUF files, the format most community quantizations are \
                 published in, so it offers the widest choice of models and sizes. It starts in \
                 seconds. On the same model, it is slower per token than Edge-LLM."
            }
            Engine::Vllm => {
                "vLLM serves Hugging Face checkpoints, including NVFP4 ones, from a Docker \
                 container and downloads the model itself on first start. It reads long prompts \
                 in chunks (chunked prefill), so no prompt length is fixed in advance. Prefix \
                 caching reuses the part of a prompt an agent sends again on every step, and an \
                 MTP draft model (speculative decoding) raises its speed. On the same model, it \
                 is slower per token than Edge-LLM. Choose it for long prompts and agent runs."
            }
        }
    }

    /// The Hugging Face tag that marks a repository in this engine's format. It is also the
    /// `filter` of the Hub search.
    pub fn format_tag(self) -> &'static str {
        match self {
            Engine::EdgeLlm => "modelopt",
            Engine::LlamaCpp => "gguf",
            Engine::Vllm => "safetensors",
        }
    }

    /// The end of a file name that marks this engine's format in a file list.
    pub fn format_file(self) -> &'static str {
        match self {
            Engine::EdgeLlm => "hf_quant_config.json",
            Engine::LlamaCpp => ".gguf",
            Engine::Vllm => ".safetensors",
        }
    }

    /// The systemd user service that runs this engine.
    pub fn service(self) -> &'static str {
        match self {
            Engine::EdgeLlm => "thor-edge-llm",
            Engine::LlamaCpp => "thor-chat",
            Engine::Vllm => "thor-vllm",
        }
    }

    /// The settings key holding this engine's port.
    pub fn port_key(self) -> &'static str {
        match self {
            Engine::EdgeLlm => "EDGE_PORT",
            Engine::LlamaCpp => "MODEL_PORT",
            Engine::Vllm => "VLLM_PORT",
        }
    }

    /// The settings key naming the model this engine serves.
    pub fn model_key(self) -> &'static str {
        match self {
            Engine::EdgeLlm => "EDGE_MODEL",
            Engine::LlamaCpp => "MODEL",
            Engine::Vllm => "VLLM_MODEL",
        }
    }

    /// The settings key holding the program this engine runs; vLLM runs `docker`.
    pub fn program_key(self) -> &'static str {
        match self {
            Engine::EdgeLlm => "EDGE_SERVER",
            Engine::LlamaCpp => "SERVER",
            Engine::Vllm => "VLLM_IMAGE",
        }
    }

    /// The settings key of the folder this engine's models are kept in.
    pub fn models_key(self) -> &'static str {
        match self {
            Engine::EdgeLlm => "EDGE_ROOT",
            Engine::LlamaCpp => "GGUF_ROOT",
            Engine::Vllm => "HF_HUB_CACHE",
        }
    }

    /// Replies the engine serves at once, each with its own KV cache: llama-server keeps
    /// `USERS` slots; the Edge-LLM service is started with a batch of one; vLLM shares one
    /// pool across replies.
    pub fn slots(self, settings: &Settings) -> u64 {
        match self {
            Engine::LlamaCpp => settings.number("USERS").unwrap_or(1).max(1),
            Engine::EdgeLlm | Engine::Vllm => 1,
        }
    }

    /// The command that serves a model on this engine, for `thor engines`.
    pub fn example(self) -> String {
        format!(
            "thor serve --engine {} --model ORGANIZATION/REPOSITORY --context max",
            self.flag()
        )
    }

    /// Whether this engine is installed, by looking for its program.
    pub fn installed(self, settings: &Settings, search_path: &str) -> Installed {
        match self {
            Engine::Vllm => docker_installed(settings, search_path),
            Engine::EdgeLlm | Engine::LlamaCpp => {
                file_installed(&settings.path_of(self.program_key()))
            }
        }
    }
}

impl fmt::Display for Engine {
    /// Writes the engine's command-line name.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.flag())
    }
}

impl Installed {
    /// `installed` or `not installed`, for a menu row.
    pub fn short(&self) -> &'static str {
        match self {
            Installed::Yes(_) => "installed",
            Installed::No(_) => "not installed",
        }
    }
}

impl fmt::Display for Installed {
    /// Writes `installed: WHERE` or `not installed: WHAT IS MISSING`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Installed::Yes(found) => write!(formatter, "installed: {found}"),
            Installed::No(missing) => write!(formatter, "not installed: {missing}"),
        }
    }
}

/// The engine a repository or folder is meant for, from its tags and file names.
pub fn engine_for(tags: &[String], files: &[String]) -> Option<Engine> {
    Engine::ALL.into_iter().find(|engine| {
        let tagged = tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(engine.format_tag()));
        let has_file = files
            .iter()
            .any(|file| file.ends_with(engine.format_file()));

        tagged || has_file
    })
}

/// Installed when the program file exists.
fn file_installed(program: &Path) -> Installed {
    if program.is_file() {
        return Installed::Yes(program.display().to_string());
    }

    Installed::No(format!("{} not found", program.display()))
}

/// Installed when `docker` is on the PATH and `VLLM_IMAGE` names an image.
fn docker_installed(settings: &Settings, search_path: &str) -> Installed {
    let docker = find_program("docker", search_path);
    let image = settings.value("VLLM_IMAGE");

    match (docker, image.is_empty()) {
        (Some(docker), false) => Installed::Yes(format!("{} with image {image}", docker.display())),
        (Some(_), true) => Installed::No("docker found, but VLLM_IMAGE is not set".to_string()),
        (None, _) => Installed::No("docker not found on the PATH".to_string()),
    }
}

/// The first file called `program` in the folders of a PATH value.
pub fn find_program(program: &str, search_path: &str) -> Option<PathBuf> {
    std::env::split_paths(search_path)
        .map(|folder| folder.join(program))
        .find(|candidate| candidate.is_file())
}
