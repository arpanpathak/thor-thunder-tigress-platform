//! Reading and editing `~/.config/thor-chat/env`, with a backup.

use std::{fs, path::Path};

use anyhow::{Result, anyhow};
use thor_tigress_cli::{
    plan::with_max_model_len,
    settings::{Settings, as_thor_tigress_serve_reads, without_surrounding_quotes},
};

/// A settings file as people write it: comments, quotes, `~`, and a key set twice.
const ENV: &str = "\
# models on the Thor
EDGE_MODEL=~/models/edge-llm/Qwen3.6-35B-A3B-NVFP4
EDGE_CONTEXT=32768
EDGE_CONTEXT=\"65536\"
VLLM_ARGS='--gpu-memory-utilization 0.3'
";

#[test]
fn the_last_line_wins_and_quotes_go() {
    let settings = Settings::parse(Path::new("/home/me/env"), Path::new("/home/me"), ENV);

    assert_eq!(settings.get("EDGE_CONTEXT").as_deref(), Some("65536"));
    assert_eq!(
        settings.get("VLLM_ARGS").as_deref(),
        Some("--gpu-memory-utilization 0.3")
    );
    assert_eq!(settings.get("# models on the Thor"), None);
}

#[test]
fn a_tilde_means_the_home_folder() {
    let settings = Settings::parse(Path::new("/home/me/env"), Path::new("/home/me"), ENV);

    assert_eq!(
        settings.get("EDGE_MODEL").as_deref(),
        Some("/home/me/models/edge-llm/Qwen3.6-35B-A3B-NVFP4")
    );
}

#[test]
fn missing_keys_fall_back_to_the_defaults() {
    let settings = Settings::parse(Path::new("/home/me/env"), Path::new("/home/me"), "");

    assert_eq!(settings.value("EDGE_PORT"), "8081");
    assert_eq!(settings.number("USERS"), Some(4));
    assert_eq!(
        settings.value("EDGE_SERVER"),
        "/home/me/.local/share/edge-llm/venv/bin/tensorrt-edgellm-serve"
    );
    assert_eq!(settings.value("NOT_A_SETTING"), "");
}

#[test]
fn set_replaces_the_last_line_and_appends_new_keys() {
    let mut settings = Settings::parse(Path::new("/home/me/env"), Path::new("/home/me"), ENV);

    settings.set("EDGE_CONTEXT", "262144");
    settings.set("EDGE_PROMPT", "253952");

    assert_eq!(
        settings.text(),
        "\
# models on the Thor
EDGE_MODEL=~/models/edge-llm/Qwen3.6-35B-A3B-NVFP4
EDGE_CONTEXT=32768
EDGE_CONTEXT=262144
VLLM_ARGS='--gpu-memory-utilization 0.3'
EDGE_PROMPT=253952
"
    );
}

#[test]
fn saving_backs_up_the_old_file_first() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join(".config/thor-chat/env");
    fs::create_dir_all(path.parent().ok_or_else(|| anyhow!("no parent"))?)?;
    fs::write(&path, ENV)?;

    let mut settings = Settings::load(&path, home.path())?;
    settings.set("EDGE_CONTEXT", "131072");
    let backup = settings
        .save_with_backup(1_763_200_000)?
        .ok_or_else(|| anyhow!("no backup"))?;

    assert_eq!(
        backup,
        home.path().join(".config/thor-chat/env.backup-1763200000")
    );
    assert_eq!(fs::read_to_string(&backup)?, ENV);
    assert!(fs::read_to_string(&path)?.contains("EDGE_CONTEXT=131072\n"));

    Ok(())
}

#[test]
fn a_new_file_needs_no_backup() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join(".config/thor-chat/env");

    let mut settings = Settings::load(&path, home.path())?;
    settings.set("MODEL", "/models/a.gguf");

    assert_eq!(settings.save_with_backup(1)?, None);
    assert_eq!(fs::read_to_string(&path)?, "MODEL=/models/a.gguf\n");

    Ok(())
}

/// The `VLLM_ARGS` serving Gemma 4 31B on the Thor, with the JSON option moved to the end,
/// where `thor-tigress-serve` breaks it.
const JSON_LAST: &str = r#"--max-model-len 262144 --gpu-memory-utilization 0.5 --speculative-config '{"method":"mtp","model":"google/gemma-4-31B-it-assistant","num_speculative_tokens":3}'"#;

#[test]
fn only_a_matching_pair_of_quotes_is_removed() {
    assert_eq!(without_surrounding_quotes("\"65536\""), "65536");
    assert_eq!(without_surrounding_quotes("'--a b'"), "--a b");
    assert_eq!(without_surrounding_quotes(JSON_LAST), JSON_LAST);
    assert_eq!(without_surrounding_quotes("'"), "'");
}

#[test]
fn a_value_ending_in_quoted_json_is_read_as_written() {
    let text = format!("VLLM_ARGS={JSON_LAST}\n");
    let settings = Settings::parse(Path::new("/env"), Path::new("/home"), &text);

    assert_eq!(settings.get("VLLM_ARGS").as_deref(), Some(JSON_LAST));
    assert_ne!(as_thor_tigress_serve_reads(JSON_LAST), JSON_LAST);
}

#[test]
fn written_vllm_args_survive_thor_tigress_serves_reader() {
    let written = with_max_model_len(JSON_LAST, 131_072);

    assert_eq!(
        written,
        r#"--gpu-memory-utilization 0.5 --speculative-config '{"method":"mtp","model":"google/gemma-4-31B-it-assistant","num_speculative_tokens":3}' --max-model-len 131072"#
    );
    assert_eq!(as_thor_tigress_serve_reads(&written), written);
}

#[test]
fn spaces_inside_json_are_kept() {
    let written = with_max_model_len(r#"--speculative-config '{"method": "mtp"}'"#, 4096);

    assert_eq!(
        written,
        r#"--speculative-config '{"method": "mtp"}' --max-model-len 4096"#
    );
}

#[test]
fn a_folder_in_place_of_the_settings_file_is_named() -> Result<()> {
    let home = tempfile::tempdir()?;

    let outcome = Settings::load(home.path(), home.path()).map(drop);

    assert!(outcome.err().is_some_and(|error| {
        error
            .to_string()
            .starts_with(&home.path().display().to_string())
    }));

    Ok(())
}

#[test]
fn a_commented_line_sets_nothing() {
    let settings = Settings::parse(
        Path::new("/home/thor/env"),
        Path::new("/home/thor"),
        "# EDGE_CONTEXT=4096\n",
    );

    assert_eq!(settings.get("EDGE_CONTEXT"), None);
}
