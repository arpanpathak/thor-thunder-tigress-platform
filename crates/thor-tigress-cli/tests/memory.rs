//! The memory estimate, from the real `config.json` of `nvidia/Gemma-4-31B-IT-NVFP4`.

use anyhow::Result;
use serde_json::{Value, json};
use thor_tigress_cli::memory::{
    self, Budget, KvShape, available_bytes, context_choices, gigabytes, tokens, total_bytes,
};

/// The Gemma 4 31B NVFP4 config, as the Hub serves it.
const GEMMA_CONFIG: &str = include_str!("fixtures/gemma-4-31b-it-nvfp4.config.json");

/// A `/proc/meminfo` from a 64 GB machine.
const MEMINFO: &str =
    "MemTotal:       64328148 kB\nMemFree:         1024000 kB\nMemAvailable:   41943040 kB\n";

/// The Gemma config, parsed.
fn gemma() -> Result<KvShape> {
    let config: Value = serde_json::from_str(GEMMA_CONFIG)?;

    Ok(KvShape::from_config(&config))
}

#[test]
fn gemma_shape_comes_from_text_config() -> Result<()> {
    let shape = gemma()?;

    assert_eq!(shape.max_context, Some(262_144));
    assert_eq!(shape.sliding_window, 1024);
    assert_eq!(shape.full_bytes_per_token, 10 * 2 * 4 * 512);
    assert_eq!(shape.sliding_bytes_per_token, 50 * 2 * 16 * 256);

    Ok(())
}

#[test]
fn sliding_layers_stop_growing_at_their_window() -> Result<()> {
    let shape = gemma()?;
    let window_part = 50 * 2 * 16 * 256 * 1024;

    assert_eq!(shape.cache_bytes(512, 1), (40_960 + 409_600) * 512);
    assert_eq!(
        shape.cache_bytes(262_144, 1),
        40_960 * 262_144 + window_part
    );
    assert_eq!(gigabytes(shape.cache_bytes(262_144, 1)), "11.2 GB");

    Ok(())
}

#[test]
fn slots_multiply_the_cache() -> Result<()> {
    let shape = gemma()?;

    assert_eq!(
        shape.cache_bytes(32_768, 4),
        4 * shape.cache_bytes(32_768, 1)
    );

    Ok(())
}

#[test]
fn an_unquantized_cache_uses_two_bytes_per_number() {
    let config = json!({
        "num_hidden_layers": 4,
        "num_attention_heads": 8,
        "num_key_value_heads": 2,
        "hidden_size": 1024,
        "max_position_embeddings": 4096
    });
    let shape = KvShape::from_config(&config);

    assert_eq!(shape.full_bytes_per_token, 4 * 2 * 2 * 128 * 2);
    assert_eq!(shape.sliding_bytes_per_token, 0);
    assert_eq!(shape.max_context, Some(4096));
}

#[test]
fn hybrid_mamba_models_count_only_attention_layers() {
    let config = json!({
        "num_hidden_layers": 6,
        "hybrid_override_pattern": "M-M*-M*",
        "num_attention_heads": 32,
        "num_key_value_heads": 2,
        "head_dim": 128
    });
    let shape = KvShape::from_config(&config);

    assert_eq!(shape.full_bytes_per_token, 2 * 2 * 2 * 128 * 2);
}

#[test]
fn linear_attention_layers_are_not_counted() {
    let config = json!({
        "num_hidden_layers": 4,
        "layer_types": ["linear_attention", "linear_attention", "linear_attention", "full_attention"],
        "num_attention_heads": 16,
        "num_key_value_heads": 2,
        "head_dim": 256
    });

    assert_eq!(
        KvShape::from_config(&config).full_bytes_per_token,
        2 * 2 * 256 * 2
    );
}

#[test]
fn meminfo_fields_are_read_in_bytes() {
    assert_eq!(available_bytes(MEMINFO), Some(41_943_040 * 1024));
    assert_eq!(total_bytes(MEMINFO), Some(64_328_148 * 1024));
    assert_eq!(available_bytes("MemTotal: 1 kB\n"), None);
}

#[test]
fn the_budget_adds_the_restarted_service_and_keeps_some_free() {
    let budget = Budget {
        available: 30_000_000_000,
        freed_by_restart: 20_000_000_000,
        keep_free: 8_000_000_000,
        service_limit: None,
    };

    assert_eq!(budget.usable(), 42_000_000_000);
    assert!(budget.fits(42_000_000_000));
    assert!(!budget.fits(42_000_000_001));
}

#[test]
fn the_service_limit_caps_the_budget() {
    let budget = Budget {
        available: 100_000_000_000,
        freed_by_restart: 0,
        keep_free: 0,
        service_limit: Some(80_000_000_000),
    };

    assert_eq!(budget.usable(), 80_000_000_000);
}

#[test]
fn context_choices_halve_down_to_the_smallest() {
    assert_eq!(
        context_choices(262_144, 16_384),
        vec![262_144, 131_072, 65_536, 32_768, 16_384]
    );
    assert_eq!(context_choices(100, 200), Vec::<u64>::new());
}

#[test]
fn numbers_read_well() {
    assert_eq!(tokens(262_144), "262,144");
    assert_eq!(tokens(1_048_576), "1,048,576");
    assert_eq!(tokens(512), "512");
    assert_eq!(memory::or_unknown(None, tokens), "?");
}

#[test]
fn nemotron_omni_keeps_its_language_model_in_llm_config() {
    let config = json!({
        "sound_config": {"num_hidden_layers": 24, "num_attention_heads": 8},
        "llm_config": {
            "num_hidden_layers": 52,
            "hybrid_override_pattern": "MEMEM*EMEMEM*EMEMEM*EMEMEM*EMEMEM*EMEMEMEM*EMEMEMEME",
            "max_position_embeddings": 262144,
            "num_attention_heads": 32,
            "num_key_value_heads": 2,
            "head_dim": 128
        },
        "quantization_config": {"kv_cache_scheme": {"num_bits": 8}}
    });
    let shape = KvShape::from_config(&config);

    assert_eq!(shape.max_context, Some(262_144));
    assert_eq!(shape.full_bytes_per_token, 6 * 2 * 2 * 128);
}
