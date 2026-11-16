//! Where a model goes, what the settings call it, and which settings change.

use std::path::{Path, PathBuf};

use serde_json::Value;
use thor_tigress_cli::{
    catalog::{Candidate, Source},
    engine::Engine,
    hub::RepoFile,
    plan::{placement, served_name, settings_changes, with_max_model_len},
    settings::Settings,
};

/// A model on the Hub with the given files.
fn on_hub(engine: Engine, repo: &str, files: &[&str]) -> Candidate {
    Candidate {
        engine,
        source: Source::Hub,
        repo: Some(repo.to_string()),
        weights: Some(1),
        shape: None,
        family: None,
        config: None::<Value>,
        downloads: Some(0),
        files: files
            .iter()
            .map(|path| RepoFile {
                kind: "file".to_string(),
                path: path.to_string(),
                size: 1,
            })
            .collect(),
    }
}

#[test]
fn an_edge_checkpoint_goes_to_a_folder_named_after_the_repository() {
    let candidate = on_hub(
        Engine::EdgeLlm,
        "nvidia/Gemma-4-31B-IT-NVFP4",
        &["config.json"],
    );
    let (target, folder) = placement(&candidate, Path::new("/m/edge-llm"));

    assert_eq!(target, "/m/edge-llm/Gemma-4-31B-IT-NVFP4");
    assert_eq!(
        folder,
        Some(PathBuf::from("/m/edge-llm/Gemma-4-31B-IT-NVFP4"))
    );
    assert_eq!(
        served_name(Engine::EdgeLlm, &target),
        "Gemma-4-31B-IT-NVFP4"
    );
}

#[test]
fn a_gguf_goes_to_a_folder_without_the_gguf_suffix() {
    let candidate = on_hub(
        Engine::LlamaCpp,
        "unsloth/gemma-4-31B-it-GGUF",
        &[
            "BF16/gemma-4-31B-it-BF16-00001-of-00002.gguf",
            "BF16/gemma-4-31B-it-BF16-00002-of-00002.gguf",
        ],
    );
    let (target, folder) = placement(&candidate, Path::new("/m/gguf"));

    assert_eq!(
        target,
        "/m/gguf/gemma-4-31B-it/BF16/gemma-4-31B-it-BF16-00001-of-00002.gguf"
    );
    assert_eq!(folder, Some(PathBuf::from("/m/gguf/gemma-4-31B-it")));
    assert_eq!(
        served_name(Engine::LlamaCpp, &target),
        "gemma-4-31B-it-BF16"
    );
}

#[test]
fn vllm_needs_only_the_repository() {
    let candidate = on_hub(Engine::Vllm, "google/gemma-4-31B-it", &[]);
    let (target, folder) = placement(&candidate, Path::new("/cache"));

    assert_eq!(target, "google/gemma-4-31B-it");
    assert_eq!(folder, None);
    assert_eq!(served_name(Engine::Vllm, &target), "gemma-4-31B-it");
}

#[test]
fn a_model_on_disk_stays_where_it_is() {
    let mut candidate = on_hub(Engine::EdgeLlm, "x/y", &[]);
    candidate.source = Source::Disk(PathBuf::from("/m/edge-llm/Local-NVFP4"));
    candidate.repo = None;

    assert_eq!(
        placement(&candidate, Path::new("/m/edge-llm")),
        ("/m/edge-llm/Local-NVFP4".to_string(), None)
    );
}

#[test]
fn edge_settings_keep_room_for_the_reply() {
    let settings = Settings::parse(Path::new("/env"), Path::new("/home"), "");
    let changes = settings_changes(&settings, Engine::EdgeLlm, "/m/G", "G", 262_144, 8192);

    assert_eq!(
        changes,
        vec![
            ("EDGE_MODEL", "/m/G".to_string()),
            ("EDGE_CONTEXT", "262144".to_string()),
            ("EDGE_PROMPT", "253952".to_string()),
        ]
    );
}

#[test]
fn llama_settings_name_the_file_and_the_context() {
    let settings = Settings::parse(Path::new("/env"), Path::new("/home"), "");
    let changes = settings_changes(&settings, Engine::LlamaCpp, "/m/a.gguf", "a", 65_536, 8192);

    assert_eq!(
        changes,
        vec![
            ("MODEL", "/m/a.gguf".to_string()),
            ("CONTEXT", "65536".to_string())
        ]
    );
}

#[test]
fn vllm_settings_keep_the_other_options() {
    let settings = Settings::parse(
        Path::new("/env"),
        Path::new("/home"),
        "VLLM_ARGS=--max-model-len 4096 --gpu-memory-utilization 0.3\n",
    );
    let changes = settings_changes(&settings, Engine::Vllm, "org/M", "M", 32_768, 8192);

    assert_eq!(
        changes,
        vec![
            ("VLLM_MODEL", "org/M".to_string()),
            ("VLLM_NAME", "M".to_string()),
            (
                "VLLM_ARGS",
                "--gpu-memory-utilization 0.3 --max-model-len 32768".to_string()
            ),
        ]
    );
}

#[test]
fn max_model_len_is_replaced_in_either_form() {
    assert_eq!(with_max_model_len("", 8192), "--max-model-len 8192");
    assert_eq!(
        with_max_model_len("--max-model-len=4096 --enforce-eager", 8192),
        "--enforce-eager --max-model-len 8192"
    );
}
