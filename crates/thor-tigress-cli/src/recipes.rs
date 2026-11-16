//! The recipes and documentation each engine's makers publish for a model family.
//!
//! The lists are fetched each time `thor serve` runs, so a newly published recipe appears
//! on the next run. vLLM keeps one recipe page per family in its `recipes` repository; NVIDIA lists
//! the checkpoints Edge-LLM supports on one page; llama.cpp publishes no per-model recipes,
//! so the model's own page is shown.

use std::time::Duration;

use serde_json::Value;

use crate::{catalog::Candidate, engine::Engine, web};

/// Every file in vLLM's recipes repository.
const VLLM_RECIPES_TREE: &str =
    "https://api.github.com/repos/vllm-project/recipes/git/trees/main?recursive=1";

/// Where a vLLM recipe page is read.
const VLLM_RECIPES_PAGE: &str = "https://github.com/vllm-project/recipes/blob/main";

/// NVIDIA's list of the checkpoints TensorRT Edge-LLM supports.
const EDGE_SUPPORTED_MODELS: &str =
    "https://nvidia.github.io/TensorRT-Edge-LLM/user_guide/getting_started/supported-models.html";

/// llama-server's documentation: its options, including context and slots.
const LLAMA_SERVER_DOCS: &str =
    "https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md";

/// The lines `thor serve` prints about the engine's documentation for this model.
/// `model_page` is the model's Hub page. A page that cannot be fetched is named, not hidden.
pub fn lines(engine: Engine, candidate: &Candidate, model_page: Option<String>) -> Vec<String> {
    let client = web::client(Duration::from_secs(10), Duration::from_secs(20));
    let family = candidate.family.as_deref().unwrap_or_default();
    let name = candidate.name();

    let mut found = match engine {
        Engine::Vllm => vllm_lines(&client, family),
        Engine::EdgeLlm => edge_lines(&client, &name),
        Engine::LlamaCpp => vec![format!("llama-server options: {LLAMA_SERVER_DOCS}")],
    };
    found.extend(model_page.map(|page| format!("model card: {page}")));

    found
}

/// vLLM's recipe pages for the family, or a note that there is none.
fn vllm_lines(client: &ureq::Agent, family: &str) -> Vec<String> {
    let Ok(tree) = web::get_text(client, VLLM_RECIPES_TREE, None) else {
        return vec![format!(
            "could not read vLLM's recipe list ({VLLM_RECIPES_TREE})"
        )];
    };
    let pages = matching_recipes(&tree, family);

    if pages.is_empty() {
        return vec![format!(
            "vLLM publishes no recipe for the {family:?} family"
        )];
    }

    pages
        .iter()
        .map(|path| format!("vLLM recipe: {VLLM_RECIPES_PAGE}/{path}"))
        .collect()
}

/// Whether NVIDIA's supported-models page lists this checkpoint, by repository or folder
/// name.
fn edge_lines(client: &ureq::Agent, repo: &str) -> Vec<String> {
    let Ok(page) = web::get_text(client, EDGE_SUPPORTED_MODELS, None) else {
        return vec![format!("could not read {EDGE_SUPPORTED_MODELS}")];
    };

    vec![
        edge_listing(&page, repo),
        format!("supported models: {EDGE_SUPPORTED_MODELS}"),
    ]
}

/// One line saying whether NVIDIA's supported-models page names the repository.
pub fn edge_listing(page: &str, repo: &str) -> String {
    let listed = !repo.is_empty() && page.to_lowercase().contains(&repo.to_lowercase());

    if listed {
        return format!("{repo} is on Edge-LLM's supported-models list");
    }

    format!("{repo} is not on Edge-LLM's supported-models list; it may still build")
}

/// The recipe pages in vLLM's repository tree whose name matches the family: `gemma4`
/// matches `Google/Gemma4.md`, and `qwen3_5_moe` matches `Qwen/Qwen3.5.md`.
pub fn matching_recipes(tree_json: &str, family: &str) -> Vec<String> {
    let tree: Value = serde_json::from_str(tree_json).unwrap_or(Value::Null);
    let wanted = letters_and_digits(family);

    tree.get("tree")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("path").and_then(Value::as_str))
        .filter(|path| is_recipe_page(path))
        .filter(|path| names_family(path, &wanted))
        .map(str::to_string)
        .collect()
}

/// A recipe page is a Markdown file one folder deep, outside the hidden folders.
fn is_recipe_page(path: &str) -> bool {
    path.ends_with(".md") && path.matches('/').count() == 1 && !path.starts_with('.')
}

/// Whether a page's file name and the family contain one another, ignoring case and
/// punctuation. Names shorter than three characters never match.
fn names_family(path: &str, wanted: &str) -> bool {
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md");
    let page = letters_and_digits(stem);
    let long_enough = page.len().min(wanted.len()) >= 3;

    long_enough
        && [(wanted, page.as_str()), (page.as_str(), wanted)]
            .iter()
            .any(|(outer, inner)| outer.contains(inner))
}

/// The text in lower case, with only letters and digits.
fn letters_and_digits(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}
