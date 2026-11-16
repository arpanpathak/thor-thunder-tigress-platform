//! Reading the Hub's replies, recorded from huggingface.co, and telling formats apart.

use anyhow::{Result, anyhow};
use thor_tigress_cli::{
    catalog::{gguf_variants, model_id},
    engine::{Engine, engine_for},
    hub::{parse_files, parse_search, search_url},
};

/// `filter=modelopt&search=gemma-4-31b`.
const SEARCH_MODELOPT: &str = include_str!("fixtures/search_modelopt.json");

/// `filter=gguf&search=gemma-4-31b`.
const SEARCH_GGUF: &str = include_str!("fixtures/search_gguf.json");

/// `filter=safetensors&search=gemma-4-31B-it`.
const SEARCH_SAFETENSORS: &str = include_str!("fixtures/search_safetensors.json");

/// The files of `nvidia/Gemma-4-31B-IT-NVFP4`.
const TREE_NVFP4: &str = include_str!("fixtures/tree_nvfp4.json");

/// The files of `unsloth/gemma-4-31B-it-GGUF`, folders included.
const TREE_GGUF: &str = include_str!("fixtures/tree_gguf.json");

#[test]
fn a_search_reply_gives_ids_downloads_and_base_models() -> Result<()> {
    let models = parse_search(SEARCH_MODELOPT)?;
    let first = models.first().ok_or_else(|| anyhow!("no results"))?;

    assert_eq!(models.len(), 3);
    assert_eq!(first.id, "nvidia/Gemma-4-31B-IT-NVFP4");
    assert_eq!(first.downloads, 1_126_472);
    assert_eq!(first.base_model(), Some("google/gemma-4-31B-it"));
    assert_eq!(first.gguf, None);

    Ok(())
}

#[test]
fn a_gguf_search_reply_gives_the_context_length() -> Result<()> {
    let models = parse_search(SEARCH_GGUF)?;
    let first = models.first().ok_or_else(|| anyhow!("no results"))?;
    let context = first.gguf.as_ref().and_then(|gguf| gguf.context_length);

    assert_eq!(first.id, "unsloth/gemma-4-31B-it-GGUF");
    assert_eq!(context, Some(262_144));

    Ok(())
}

#[test]
fn tags_name_the_engine() -> Result<()> {
    let modelopt = parse_search(SEARCH_MODELOPT)?;
    let gguf = parse_search(SEARCH_GGUF)?;
    let plain = parse_search(SEARCH_SAFETENSORS)?;
    let google = plain
        .iter()
        .find(|model| model.id == "google/gemma-4-31B-it")
        .ok_or_else(|| anyhow!("google/gemma-4-31B-it missing"))?;

    assert!(
        modelopt
            .iter()
            .all(|model| engine_for(&model.tags, &[]) == Some(Engine::EdgeLlm))
    );
    assert!(
        gguf.iter()
            .all(|model| engine_for(&model.tags, &[]) == Some(Engine::LlamaCpp))
    );
    assert_eq!(engine_for(&google.tags, &[]), Some(Engine::Vllm));

    Ok(())
}

#[test]
fn file_names_name_the_engine_when_there_are_no_tags() -> Result<()> {
    let names = |tree: &str| -> Result<Vec<String>> {
        Ok(parse_files(tree)?
            .into_iter()
            .map(|file| file.path)
            .collect())
    };
    let plain = vec!["config.json".to_string(), "model.safetensors".to_string()];

    assert_eq!(engine_for(&[], &names(TREE_NVFP4)?), Some(Engine::EdgeLlm));
    assert_eq!(engine_for(&[], &names(TREE_GGUF)?), Some(Engine::LlamaCpp));
    assert_eq!(engine_for(&[], &plain), Some(Engine::Vllm));
    assert_eq!(engine_for(&[], &["README.md".to_string()]), None);

    Ok(())
}

#[test]
fn vllm_also_serves_quantized_checkpoints() {
    assert!(Engine::Vllm.can_serve(Engine::EdgeLlm));
    assert!(Engine::EdgeLlm.can_serve(Engine::EdgeLlm));
    assert!(!Engine::EdgeLlm.can_serve(Engine::LlamaCpp));
    assert!(!Engine::LlamaCpp.can_serve(Engine::Vllm));
}

#[test]
fn engine_flags_round_trip() -> Result<()> {
    for engine in Engine::MENU {
        assert_eq!(Engine::from_flag(engine.flag())?, engine);
    }

    assert!(Engine::from_flag("tensorrt-llm").is_err());

    Ok(())
}

#[test]
fn the_file_list_keeps_files_and_sizes() -> Result<()> {
    let files = parse_files(TREE_NVFP4)?;
    let weights: u64 = files
        .iter()
        .filter(|file| file.path.ends_with(".safetensors"))
        .map(|file| file.size)
        .sum();

    assert!(files.iter().any(|file| file.path == "hf_quant_config.json"));
    assert_eq!(weights, 32_633_477_808);

    Ok(())
}

#[test]
fn gguf_variants_group_split_files_and_leave_out_helpers() -> Result<()> {
    let files = parse_files(TREE_GGUF)?;
    let variants = gguf_variants(&files);
    let bf16 = variants
        .iter()
        .find(|variant| variant.name == "gemma-4-31B-it-BF16")
        .ok_or_else(|| anyhow!("no BF16 variant"))?;
    let smallest = variants.first().ok_or_else(|| anyhow!("no variants"))?;

    assert_eq!(bf16.files.len(), 2);
    assert_eq!(bf16.bytes, 49_950_156_960 + 11_463_033_568);
    assert_eq!(smallest.name, "gemma-4-31B-it-UD-IQ2_XXS");
    assert!(
        !variants
            .iter()
            .any(|variant| variant.name.contains("mmproj"))
    );
    assert!(
        !variants
            .iter()
            .any(|variant| variant.name.starts_with("mtp"))
    );
    assert_eq!(variants.len(), 22);

    Ok(())
}

#[test]
fn model_ids_drop_folders_and_part_suffixes() {
    assert_eq!(
        model_id("BF16/gemma-4-31B-it-BF16-00001-of-00002.gguf"),
        "gemma-4-31B-it-BF16"
    );
    assert_eq!(
        model_id("gemma-4-31B-it-Q4_K_M.gguf"),
        "gemma-4-31B-it-Q4_K_M"
    );
}

#[test]
fn the_search_url_filters_by_format_and_escapes_the_words() {
    let url = search_url("https://hub.example", Engine::EdgeLlm, "gemma 4/31b", 5);

    assert!(
        url.starts_with("https://hub.example/api/models?filter=modelopt&search=gemma%204%2F31b")
    );
    assert!(url.contains("&limit=5"));
    assert!(url.contains("expand[]=baseModels"));
}

#[test]
fn paths_that_leave_the_download_folder_are_dropped() -> Result<()> {
    let reply = r#"[
        {"type":"file","path":"config.json","size":1},
        {"type":"file","path":"../../.bashrc","size":1},
        {"type":"file","path":"/etc/passwd","size":1},
        {"type":"file","path":"BF16/part.gguf","size":1}
    ]"#;
    let paths: Vec<String> = parse_files(reply)?
        .into_iter()
        .map(|file| file.path)
        .collect();

    assert_eq!(paths, vec!["config.json", "BF16/part.gguf"]);

    Ok(())
}
