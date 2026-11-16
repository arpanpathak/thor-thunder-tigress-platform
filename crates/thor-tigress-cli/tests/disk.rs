//! Finding models on disk, in each engine's own folder layout.

use std::{fs, path::Path};

use anyhow::{Result, anyhow};
use thor_tigress_cli::{
    catalog::{Candidate, from_disk, on_disk},
    engine::Engine,
};

/// The Gemma 4 31B NVFP4 config.
const GEMMA_CONFIG: &str = include_str!("fixtures/gemma-4-31b-it-nvfp4.config.json");

/// Writes `size` bytes to a new file, making its folder first.
fn file_of_size(path: &Path, size: usize) -> Result<()> {
    fs::create_dir_all(path.parent().ok_or_else(|| anyhow!("no parent"))?)?;
    fs::write(path, vec![0_u8; size])?;

    Ok(())
}

/// The only candidate, or an error.
fn only(candidates: Vec<Candidate>) -> Result<Candidate> {
    let count = candidates.len();

    candidates
        .into_iter()
        .next()
        .filter(|_candidate| count == 1)
        .ok_or_else(|| anyhow!("expected one candidate, found {count}"))
}

#[test]
fn edge_checkpoints_are_folders_with_a_config() -> Result<()> {
    let root = tempfile::tempdir()?;
    let checkpoint = root.path().join("Gemma-4-31B-IT-NVFP4");
    file_of_size(&checkpoint.join("model-00001-of-00002.safetensors"), 100)?;
    file_of_size(&checkpoint.join("model-00002-of-00002.safetensors"), 50)?;
    fs::write(checkpoint.join("config.json"), GEMMA_CONFIG)?;
    file_of_size(&root.path().join("drafters/config.json"), 2)?;
    file_of_size(&root.path().join("cache/config.json"), 2)?;

    let model = only(on_disk(Engine::EdgeLlm, root.path()))?;

    assert_eq!(model.name(), "Gemma-4-31B-IT-NVFP4");
    assert_eq!(model.weights, Some(150));
    assert_eq!(model.max_context(), Some(262_144));
    assert_eq!(model.family.as_deref(), Some("gemma4"));
    assert_eq!(model.cost(1024, 1), Some(150 + (40_960 + 409_600) * 1024));
    assert!(model.is_named_by(&checkpoint.display().to_string()));
    assert!(!model.is_named_by(""));

    Ok(())
}

#[test]
fn gguf_files_count_every_part_and_skip_helpers() -> Result<()> {
    let root = tempfile::tempdir()?;
    let folder = root.path().join("gemma-4-31B-it");
    file_of_size(&folder.join("gemma-4-31B-it-BF16-00001-of-00002.gguf"), 10)?;
    file_of_size(&folder.join("gemma-4-31B-it-BF16-00002-of-00002.gguf"), 20)?;
    file_of_size(&folder.join("mmproj-F16.gguf"), 5)?;
    file_of_size(&root.path().join("single-Q8_0.gguf"), 7)?;
    fs::write(folder.join("config.json"), GEMMA_CONFIG)?;

    let models = on_disk(Engine::LlamaCpp, root.path());
    let names: Vec<String> = models.iter().map(Candidate::name).collect();
    let weights: Vec<Option<u64>> = models.iter().map(|model| model.weights).collect();

    assert_eq!(names, vec!["gemma-4-31B-it-BF16", "single-Q8_0"]);
    assert_eq!(weights, vec![Some(30), Some(7)]);
    assert_eq!(
        models.first().and_then(Candidate::max_context),
        Some(262_144)
    );

    Ok(())
}

#[test]
fn the_hugging_face_cache_gives_repository_names() -> Result<()> {
    let root = tempfile::tempdir()?;
    let repo = root.path().join("models--google--gemma-4-31B-it");
    let snapshot = repo.join("snapshots/0123abcd");
    file_of_size(&repo.join("blobs/aaaa"), 30)?;
    fs::create_dir_all(&snapshot)?;
    std::os::unix::fs::symlink("../../blobs/aaaa", snapshot.join("model.safetensors"))?;
    fs::write(snapshot.join("config.json"), GEMMA_CONFIG)?;
    fs::create_dir_all(root.path().join("models--empty--repo/snapshots/1"))?;

    let model = only(on_disk(Engine::Vllm, root.path()))?;

    assert_eq!(model.name(), "google/gemma-4-31B-it");
    assert_eq!(model.weights, Some(30));
    assert!(model.is_named_by("google/gemma-4-31B-it"));

    Ok(())
}

#[test]
fn a_folder_without_a_config_has_no_shape() -> Result<()> {
    let root = tempfile::tempdir()?;
    file_of_size(&root.path().join("model.safetensors"), 9)?;

    let model = from_disk(Engine::Vllm, root.path(), None);

    assert_eq!(model.weights, Some(9));
    assert_eq!(model.max_context(), None);
    assert_eq!(model.cost(1024, 1), None);

    Ok(())
}
