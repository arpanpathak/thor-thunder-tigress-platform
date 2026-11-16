//! The command line, systemd's and the engines' replies, recipe matching, and the
//! messages people read.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use thor_tigress_cli::{
    catalog::{Candidate, Source},
    cli::{Command, ContextChoice, ServeOptions, parse},
    commands::{find_unit, wrap},
    engine::Engine,
    memory::{Budget, KvShape},
    recipes::{edge_listing, matching_recipes},
    serve::{check_window, does_not_fit_message, fit_word, largest_fitting_context},
    service::{UnitState, error_lines, parse_model_ids, parse_show, parse_unit_list},
};

/// The `.md` files of vLLM's recipes repository.
const VLLM_TREE: &str = include_str!("fixtures/vllm_recipes_tree.json");

/// The words of a command line.
fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

/// A model with Gemma 4 31B's cache shape and 20 GB of weights.
fn gemma_like() -> Candidate {
    Candidate {
        engine: Engine::EdgeLlm,
        source: Source::Disk(PathBuf::from("/m/Gemma")),
        repo: None,
        weights: Some(20_000_000_000),
        shape: Some(KvShape {
            max_context: Some(262_144),
            full_bytes_per_token: 40_960,
            sliding_bytes_per_token: 409_600,
            sliding_window: 1024,
            vocab_size: Some(262_144),
        }),
        family: Some("gemma4".to_string()),
        config: None,
        downloads: None,
        files: Vec::new(),
    }
}

/// A budget of `usable` bytes.
fn budget(usable: u64) -> Budget {
    Budget {
        available: usable,
        freed_by_restart: 0,
        keep_free: 0,
        service_limit: None,
    }
}

#[test]
fn serve_flags_are_read() -> Result<()> {
    let cli = parse(&words(
        "serve --engine edge-llm --model nvidia/Gemma-4-31B-IT-NVFP4 --context max --yes --settings /tmp/env",
    ))?;
    let expected = ServeOptions {
        engine: Some(Engine::EdgeLlm),
        model: Some("nvidia/Gemma-4-31B-IT-NVFP4".to_string()),
        file: None,
        context: Some(ContextChoice::Max),
        reply_tokens: 8192,
        limit: 12,
        wait: Duration::from_secs(3600),
        docs: true,
        yes: true,
    };

    assert_eq!(cli.command, Command::Serve(expected));
    assert_eq!(cli.settings, Some(PathBuf::from("/tmp/env")));

    Ok(())
}

#[test]
fn context_takes_a_number_with_commas() -> Result<()> {
    let cli = parse(&words("serve --context 131,072"))?;
    let Command::Serve(options) = cli.command else {
        return Err(anyhow::anyhow!("not serve"));
    };

    assert_eq!(options.context, Some(ContextChoice::Tokens(131_072)));

    Ok(())
}

#[test]
fn unknown_command_lines_get_a_usage_message() {
    assert!(parse(&words("serve --engine trt")).is_err());
    assert!(parse(&words("serve --context lots")).is_err());
    assert!(parse(&words("serve --colour red")).is_err());
    assert!(parse(&words("serve --model")).is_err());
    assert!(parse(&words("launch")).is_err());
}

#[test]
fn other_commands_are_read() -> Result<()> {
    assert_eq!(parse(&[])?.command, Command::Help);
    assert_eq!(parse(&words("serve --help"))?.command, Command::Help);
    assert_eq!(parse(&words("engines"))?.command, Command::Engines);
    assert_eq!(parse(&words("list"))?.command, Command::List);
    assert_eq!(
        parse(&words("stop thor-edge-llm -y"))?.command,
        Command::Stop {
            name: Some("thor-edge-llm".to_string()),
            yes: true
        }
    );
    assert_eq!(parse(&words("logs"))?.command, Command::Logs { name: None });

    Ok(())
}

#[test]
fn systemctl_show_is_read() {
    let state = parse_show(
        "thor-edge-llm",
        "LoadState=loaded\nActiveState=active\nMemoryCurrent=23400000000\nMemoryMax=infinity\nNRestarts=2\n",
    );

    assert_eq!(
        state,
        UnitState {
            name: "thor-edge-llm".to_string(),
            load_state: "loaded".to_string(),
            active_state: "active".to_string(),
            memory: Some(23_400_000_000),
            memory_max: None,
            restarts: 2,
            container: None,
        }
    );
    assert!(state.is_running());
    assert!(state.has_failed_since(1));
    assert!(!state.has_failed_since(2));
}

#[test]
fn unit_lists_are_read() {
    let text = "thor-chat.service loaded active running Models\nthor-edge-llm.service loaded failed failed X\n";

    assert_eq!(parse_unit_list(text), vec!["thor-chat", "thor-edge-llm"]);
}

#[test]
fn error_lines_include_tensorrt_running_out_of_memory() {
    let log = "\
Building the engine for Gemma-4-31B-IT-NVFP4
[TensorRT] Tactic 12 took 3 ms
[TensorRT] ERROR: [virtualMemoryBuffer.cpp::resizePhysical::154] Error Code 2: OutOfMemory (ran out of memory)
thor-edge-llm.service: Main process exited, code=killed, status=9/KILL
Serving on 127.0.0.1:8081";

    assert_eq!(error_lines(log).len(), 2);
    assert!(
        error_lines(log)
            .iter()
            .all(|line| !line.starts_with("Serving"))
    );
}

#[test]
fn model_ids_are_read_from_v1_models() {
    let reply = r#"{"object":"list","data":[{"id":"Gemma-4-31B-IT-NVFP4","object":"model"}]}"#;

    assert_eq!(parse_model_ids(reply), vec!["Gemma-4-31B-IT-NVFP4"]);
    assert_eq!(parse_model_ids("not json"), Vec::<String>::new());
}

#[test]
fn vllm_recipes_match_the_family() {
    assert_eq!(
        matching_recipes(VLLM_TREE, "gemma4"),
        vec!["Google/Gemma4.md"]
    );
    assert_eq!(
        matching_recipes(VLLM_TREE, "qwen3_5_moe"),
        vec!["Qwen/Qwen3.5.md", "Qwen/Qwen3.md"]
    );
    assert_eq!(matching_recipes(VLLM_TREE, ""), Vec::<String>::new());
}

#[test]
fn the_edge_listing_names_the_repository() {
    let page = "<a href=\"https://huggingface.co/nvidia/Gemma-4-31B-IT-NVFP4\">nvidia/Gemma-4-31B-IT-NVFP4</a>";

    assert!(edge_listing(page, "nvidia/Gemma-4-31B-IT-NVFP4").contains(" is on "));
    assert!(edge_listing(page, "someone/Other-NVFP4").contains(" is not on "));
}

#[test]
fn fit_words_cover_every_case() {
    let room = budget(30_000_000_000);

    assert_eq!(
        fit_word(&room, Some(29_000_000_000), Some(20_000_000_000)),
        "fits at max"
    );
    assert_eq!(
        fit_word(&room, Some(31_000_000_000), Some(20_000_000_000)),
        "fits, smaller context"
    );
    assert_eq!(
        fit_word(&room, Some(41_000_000_000), Some(40_000_000_000)),
        "too big"
    );
    assert_eq!(fit_word(&room, None, None), "size unknown");
}

#[test]
fn the_memory_message_says_what_fits_and_what_to_stop() {
    let candidate = gemma_like();
    let room = budget(26_000_000_000);
    let running = vec![UnitState {
        name: "thor-chat".to_string(),
        load_state: "loaded".to_string(),
        active_state: "active".to_string(),
        memory: Some(57_800_000_000),
        memory_max: None,
        restarts: 0,
        container: None,
    }];
    let message = does_not_fit_message(&candidate, 262_144, 1, &room, &running);

    assert_eq!(largest_fitting_context(&candidate, 1, &room), Some(131_072));
    assert!(
        message.starts_with("Gemma at 262,144 tokens needs about 31.2 GB; 26.0 GB can be used.")
    );
    assert!(message.contains("The largest context that fits is 131,072: --context 131072"));
    assert!(
        message.contains("To make room, stop thor-chat (57.8 GB): thor-tigress stop thor-chat")
    );
}

#[test]
fn the_window_must_hold_a_prompt_and_stay_under_the_maximum() {
    let candidate = gemma_like();

    assert!(check_window(&candidate, 262_144, 8192).is_ok());
    assert!(check_window(&candidate, 524_288, 8192).is_err());
    assert!(check_window(&candidate, 8192, 8192).is_err());
}

#[test]
fn units_are_found_by_name_prefix_or_engine() -> Result<()> {
    let units = vec!["thor-chat".to_string(), "thor-edge-llm".to_string()];

    assert_eq!(find_unit(&units, "thor-chat")?, "thor-chat");
    assert_eq!(find_unit(&units, "chat")?, "thor-chat");
    assert_eq!(find_unit(&units, "edge-llm")?, "thor-edge-llm");
    assert!(find_unit(&units, "vllm").is_err());

    Ok(())
}

#[test]
fn text_wraps_at_spaces() {
    let lines = wrap("one two three four five six", 10);

    assert_eq!(lines, vec!["one two", "three four", "five six"]);
}
