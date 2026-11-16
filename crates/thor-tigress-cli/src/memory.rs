//! What a model costs in memory, and how much memory there is.
//!
//! A served model holds its weights and a key/value (KV) cache. The cache holds, for every
//! token of context and every attention layer, one key and one value vector per KV head.
//! A sliding-window layer keeps only its window, so it stops growing once the context passes
//! the window. Linear-attention and Mamba layers keep a fixed state and are not counted.
//! Every number comes from the model's own `config.json`.

use serde_json::Value;

/// Bytes in one gigabyte, as `thor-tigress-serve` and the book count them.
pub const GIGABYTE: f64 = 1e9;

/// The sections in which multimodal configs keep the language model's numbers: Gemma and
/// Qwen use `text_config`, NVIDIA's Nemotron Omni uses `llm_config`.
pub const LANGUAGE_SECTIONS: [&str; 3] = ["text_config", "llm_config", "language_config"];

/// The KV cache's shape, read from a model's `config.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KvShape {
    /// The longest context the model was trained for, when the config states it.
    pub max_context: Option<u64>,
    /// Cache bytes per token across all full-attention layers.
    pub full_bytes_per_token: u64,
    /// Cache bytes per token across all sliding-window layers.
    pub sliding_bytes_per_token: u64,
    /// The sliding window in tokens; 0 when the model has none.
    pub sliding_window: u64,
    /// The vocabulary size, which sets the size of the language-model head.
    pub vocab_size: Option<u64>,
}

impl KvShape {
    /// Reads the shape from a `config.json`. Multimodal configs keep the language model's
    /// numbers in a section of their own ([`LANGUAGE_SECTIONS`]), which is read first.
    pub fn from_config(config: &Value) -> KvShape {
        let text = LANGUAGE_SECTIONS
            .iter()
            .find_map(|section| config.get(section))
            .unwrap_or(config);
        let (full_layers, sliding_layers) = layer_counts(text);

        let attention_heads = number(text, "num_attention_heads").unwrap_or(1).max(1);
        let hidden_size = number(text, "hidden_size").unwrap_or(0);
        let kv_heads = number(text, "num_key_value_heads").unwrap_or(attention_heads);
        let head_dim = number(text, "head_dim").unwrap_or(hidden_size / attention_heads);

        let global_kv_heads = number(text, "num_global_key_value_heads").unwrap_or(kv_heads);
        let global_head_dim = number(text, "global_head_dim").unwrap_or(head_dim);
        let element_bytes = kv_element_bytes(config);

        KvShape {
            max_context: number(text, "max_position_embeddings")
                .or_else(|| number(config, "max_position_embeddings")),
            full_bytes_per_token: full_layers
                * 2
                * global_kv_heads
                * global_head_dim
                * element_bytes,
            sliding_bytes_per_token: sliding_layers * 2 * kv_heads * head_dim * element_bytes,
            sliding_window: number(text, "sliding_window").unwrap_or(0),
            vocab_size: number(text, "vocab_size").or_else(|| number(config, "vocab_size")),
        }
    }

    /// The KV cache for `context` tokens in each of `slots` replies at once.
    pub fn cache_bytes(&self, context: u64, slots: u64) -> u64 {
        let sliding_tokens = context.min(self.sliding_window);
        let per_slot =
            self.full_bytes_per_token * context + self.sliding_bytes_per_token * sliding_tokens;

        per_slot * slots
    }
}

/// How many layers keep a full cache and how many keep a window.
///
/// `layer_types` names each layer (`full_attention`, `sliding_attention`, `linear_attention`).
/// Nemotron-H configs give `hybrid_override_pattern` instead, where `*` marks attention.
/// A config with neither has full attention on every layer.
fn layer_counts(text: &Value) -> (u64, u64) {
    let layer_types = text.get("layer_types").and_then(Value::as_array);
    let pattern = text.get("hybrid_override_pattern").and_then(Value::as_str);
    let layers = number(text, "num_hidden_layers").unwrap_or(0);

    match (layer_types, pattern) {
        (Some(types), _) => (
            count_named(types, "full_attention"),
            count_named(types, "sliding_attention"),
        ),
        (None, Some(pattern)) => (pattern.matches('*').count() as u64, 0),
        (None, None) => (layers, 0),
    }
}

/// How many entries of a JSON list are the string `name`.
fn count_named(types: &[Value], name: &str) -> u64 {
    types
        .iter()
        .filter(|layer_type| layer_type.as_str() == Some(name))
        .count() as u64
}

/// Bytes per cached number: the checkpoint's `kv_cache_scheme.num_bits` when it quantizes
/// the cache (NVIDIA's NVFP4 checkpoints use 8), else 2 for a 16-bit cache.
fn kv_element_bytes(config: &Value) -> u64 {
    let bits = config
        .pointer("/quantization_config/kv_cache_scheme/num_bits")
        .and_then(Value::as_u64)
        .unwrap_or(16);

    bits.div_ceil(8)
}

/// A whole number from a JSON object, when the key holds one.
fn number(object: &Value, key: &str) -> Option<u64> {
    object.get(key).and_then(Value::as_u64)
}

/// `MemAvailable` from `/proc/meminfo`, in bytes.
pub fn available_bytes(meminfo: &str) -> Option<u64> {
    meminfo_field(meminfo, "MemAvailable:")
}

/// `MemTotal` from `/proc/meminfo`, in bytes.
pub fn total_bytes(meminfo: &str) -> Option<u64> {
    meminfo_field(meminfo, "MemTotal:")
}

/// One `/proc/meminfo` field, converted from kB to bytes.
fn meminfo_field(meminfo: &str, name: &str) -> Option<u64> {
    let line = meminfo.lines().find(|line| line.starts_with(name))?;
    let kilobytes: u64 = line.split_whitespace().nth(1)?.parse().ok()?;

    Some(kilobytes * 1024)
}

/// The memory a model would use, and the memory there is for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// `MemAvailable` now.
    pub available: u64,
    /// What the service being replaced holds now, given back when it restarts.
    pub freed_by_restart: u64,
    /// Memory left free for everything else (`MIN_FREE_GB`).
    pub keep_free: u64,
    /// The service's own `MemoryMax`, when it has one.
    pub service_limit: Option<u64>,
}

impl Budget {
    /// The memory the new model may use.
    pub fn usable(&self) -> u64 {
        let free = (self.available + self.freed_by_restart).saturating_sub(self.keep_free);

        self.service_limit.map_or(free, |limit| free.min(limit))
    }

    /// Whether `bytes` fits.
    pub fn fits(&self, bytes: u64) -> bool {
        bytes <= self.usable()
    }
}

/// A size in gigabytes with one decimal, such as `20.9 GB`.
pub fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GIGABYTE)
}

/// The context sizes offered in the menu: the maximum, then halves of it down to `smallest`.
pub fn context_choices(max_context: u64, smallest: u64) -> Vec<u64> {
    std::iter::successors(Some(max_context), |context| Some(context / 2))
        .take_while(|context| *context >= smallest.max(1))
        .collect()
}

/// A token count with thousands separated by commas, such as `262,144`.
pub fn tokens(count: u64) -> String {
    let digits = count.to_string();
    let groups: Vec<&str> = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|group| std::str::from_utf8(group).unwrap_or_default())
        .collect();

    groups.join(",")
}

/// A value formatted, or `?` when it is unknown.
pub fn or_unknown(value: Option<u64>, format: fn(u64) -> String) -> String {
    value.map_or_else(|| "?".to_string(), format)
}
