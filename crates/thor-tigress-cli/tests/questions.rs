//! The questions `thor-tigress serve`, `stop` and `logs` ask, answered from a script.
//!
//! The library finds `systemctl`, `journalctl` and `docker` on the PATH and the Hub in
//! `HF_ENDPOINT`, so each test runs itself again in a child process whose environment points
//! at the fake Thor of `common`. The test process starts the fake engine and the fake Hub;
//! the child calls the library and checks what it asked and did.

mod common;

use std::{
    collections::VecDeque,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

use anyhow::{Result, anyhow};
use common::{FakeThor, GEMMA_CONFIG, fake_engine, fake_hub, free_port, text};
use inquire::InquireError;
use thor_tigress_cli::{
    cli::{ContextChoice, ServeOptions},
    commands,
    engine::Engine,
    error::{Outcome, ThorError},
    hub::{Hub, RepoFile},
    machine::Machine,
    prompt::Prompt,
    serve::serve,
    service,
    settings::Settings,
};
use Answer::{Cancel, Choose, Number, Type, YesNo};

/// Names the fake home of a child process started by [`FakeThor::run_test`].
const CHILD_HOME: &str = "THOR_TEST_HOME";

/// Enough memory for every model here: what the fake services hold and give back.
const PLENTY_OF_MEMORY: (&str, &str) = ("THOR_FAKE_MEMORY", "100000000000");

/// The Hub turned on, at the fake Hub started by [`in_child`].
const HUB_ON: (&str, &str) = ("HF_HUB_OFFLINE", "0");

/// The model menu row of the checkpoint on the fake Thor's disk.
const CHECKPOINT_ROW: &str = "Gemma-4-31B-IT-NVFP4";

/// What `thor-tigress serve` says when its last question is answered no.
const STOPPED: &str = "Stopped; nothing was changed.";

/// One scripted answer.
enum Answer {
    /// Choose the menu row whose text contains this.
    Choose(&'static str),
    /// Type this line.
    Type(&'static str),
    /// Type this number.
    Number(u64),
    /// Answer yes or no.
    YesNo(bool),
    /// Press Escape.
    Cancel,
}

/// Answers from a script, and records every question, help line and menu row shown.
struct Answers {
    /// The answers still to give, in order.
    script: VecDeque<Answer>,
    /// Everything shown so far.
    shown: Vec<String>,
}

impl Answers {
    /// Answers that give `script` in order.
    fn new<const COUNT: usize>(script: [Answer; COUNT]) -> Answers {
        Answers {
            script: VecDeque::from(script),
            shown: Vec::new(),
        }
    }

    /// The next answer, for `question`. Escape gives the error `inquire` gives.
    fn next(&mut self, question: &str) -> Outcome<Answer> {
        self.shown.push(question.to_string());

        match self.script.pop_front() {
            Some(Cancel) => Err(ThorError::Prompt(InquireError::OperationCanceled)),
            Some(answer) => Ok(answer),
            None => Err(unexpected(question)),
        }
    }

    /// Everything shown, one item per line.
    fn shown(&self) -> String {
        self.shown.join("\n")
    }
}

impl Prompt for Answers {
    /// The first row containing the scripted text.
    fn select<Row: Display>(
        &mut self,
        question: &str,
        help: Option<&str>,
        rows: Vec<Row>,
    ) -> Outcome<Row> {
        let answer = self.next(question);
        self.shown.extend(help.map(str::to_string));
        self.shown.extend(rows.iter().map(ToString::to_string));

        let Choose(wanted) = answer? else {
            return Err(unexpected(question));
        };

        rows.into_iter()
            .find(|row| row.to_string().contains(wanted))
            .ok_or_else(|| unexpected(question))
    }

    /// The scripted line.
    fn text(&mut self, question: &str) -> Outcome<String> {
        let Type(line) = self.next(question)? else {
            return Err(unexpected(question));
        };

        Ok(line.to_string())
    }

    /// The scripted number.
    fn number(&mut self, question: &str) -> Outcome<u64> {
        let Number(number) = self.next(question)? else {
            return Err(unexpected(question));
        };

        Ok(number)
    }

    /// The scripted yes or no.
    fn confirm(&mut self, question: &str, _default: bool) -> Outcome<bool> {
        let YesNo(yes) = self.next(question)? else {
            return Err(unexpected(question));
        };

        Ok(yes)
    }
}

/// The error for a question the script does not answer as it should.
fn unexpected(question: &str) -> ThorError {
    ThorError::Condition(format!("the script has no fitting answer for {question:?}"))
}

/// The fake home this process runs in when [`FakeThor::run_test`] started it; `None` in the
/// test process itself.
fn child_home() -> Option<PathBuf> {
    std::env::var_os(CHILD_HOME).map(PathBuf::from)
}

/// What only these tests ask of the fake Thor.
impl FakeThor {
    /// Runs the test named `test` again in a child process with this home, the fake
    /// programs first on the PATH, and `extra_environment`. Fails with the child's output
    /// unless exactly that test ran and passed.
    fn run_test(&self, test: &str, extra_environment: &[(&str, &str)]) -> Result<Output> {
        let mut command = self.command(&std::env::current_exe()?);
        command
            .args([test, "--exact", "--nocapture"])
            .env(CHILD_HOME, self.path())
            .envs(extra_environment.iter().copied());

        let output = command.output()?;
        let (stdout, stderr) = text(&output);

        if !stdout.contains("test result: ok. 1 passed") {
            return Err(anyhow!("{test} in a child process:\n{stdout}\n{stderr}"));
        }

        Ok(output)
    }
}

/// Starts a fake engine serving the checkpoint and a fake Hub with the tiny models, then
/// runs `test` in a child process on a new fake Thor with `extra_settings`, plenty of
/// memory, and `extra_environment`. The Hub stays off unless `extra_environment` has
/// [`HUB_ON`]. Returns the child's standard output and standard error.
fn in_child(
    test: &str,
    extra_settings: &str,
    extra_environment: &[(&str, &str)],
) -> Result<(String, String)> {
    let thor = FakeThor::new(fake_engine(CHECKPOINT_ROW)?, extra_settings)?;
    let (hub_port, _requests) = fake_hub(tiny_hub_routes())?;
    let endpoint = format!("http://127.0.0.1:{hub_port}");
    let environment = [("HF_ENDPOINT", endpoint.as_str()), PLENTY_OF_MEMORY];

    let output = thor.run_test(test, &[&environment, extra_environment].concat())?;

    Ok(text(&output))
}

/// The fake Hub's text replies: `org/tiny-GGUF` with GGUF variants, one of them a petabyte;
/// `org/tiny` with safetensors; `org/huge` with ten terabytes of them; `org/no-tree`, whose
/// file list is missing; and `org/empty-GGUF` with no GGUF files. A search returns the
/// first two.
const TINY_HUB: [(&str, &str); 10] = [
    (
        "/api/models",
        r#"[{"id":"org/tiny-GGUF","downloads":3,"tags":["gguf"],"baseModels":{"models":[{"id":"org/tiny"}]},"gguf":{"context_length":4096}},{"id":"org/tiny","downloads":9,"tags":["safetensors"]}]"#,
    ),
    (
        "/api/models/org/tiny-GGUF",
        r#"{"id":"org/tiny-GGUF","downloads":3,"tags":["gguf"],"baseModels":{"models":[{"id":"org/tiny"}]},"gguf":{"context_length":4096}}"#,
    ),
    (
        "/api/models/org/tiny",
        r#"{"id":"org/tiny","downloads":9,"tags":["safetensors"]}"#,
    ),
    (
        "/api/models/org/empty-GGUF",
        r#"{"id":"org/empty-GGUF","tags":["gguf"]}"#,
    ),
    (
        "/api/models/org/huge",
        r#"{"id":"org/huge","tags":["safetensors"]}"#,
    ),
    (
        "/api/models/org/no-tree",
        r#"{"id":"org/no-tree","tags":["safetensors"]}"#,
    ),
    (
        "/api/models/org/tiny-GGUF/tree/main",
        r#"[{"type":"file","path":"tiny-Q4_K_M.gguf","size":100},{"type":"file","path":"tiny-Q8_0.gguf","size":200},{"type":"file","path":"tiny-BF16.gguf","size":1000000000000000}]"#,
    ),
    (
        "/api/models/org/tiny/tree/main",
        r#"[{"type":"file","path":"config.json","size":10},{"type":"file","path":"model.safetensors","size":300}]"#,
    ),
    (
        "/api/models/org/empty-GGUF/tree/main",
        r#"[{"type":"file","path":"README.md","size":10}]"#,
    ),
    (
        "/api/models/org/huge/tree/main",
        r#"[{"type":"file","path":"model.safetensors","size":10000000000000}]"#,
    ),
];

/// The fake Hub: [`TINY_HUB`], the Gemma config for `org/tiny`, `org/huge` and
/// `org/no-tree`, and the bytes of `tiny-Q8_0.gguf`.
fn tiny_hub_routes() -> Vec<(String, Vec<u8>)> {
    let configs = ["org/tiny", "org/huge", "org/no-tree"].map(|repo| {
        (
            format!("/{repo}/resolve/main/config.json"),
            GEMMA_CONFIG.into(),
        )
    });
    let variant = (
        "/org/tiny-GGUF/resolve/main/tiny-Q8_0.gguf".to_string(),
        vec![8_u8; 200],
    );

    TINY_HUB
        .into_iter()
        .map(|(path, body)| (path.to_string(), body.into()))
        .chain(configs)
        .chain([variant])
        .collect()
}

/// The machine of the fake home, with its settings file.
fn fake_machine(home: &Path) -> Result<Machine> {
    Ok(Machine::load(Some(home.join("env")))?)
}

/// Options that leave every question open, for a reply of 512 tokens.
fn ask_everything() -> ServeOptions {
    ServeOptions {
        engine: None,
        model: None,
        file: None,
        context: None,
        reply_tokens: 512,
        limit: 12,
        wait: Duration::from_secs(30),
        docs: true,
        yes: false,
    }
}

/// A new Edge-LLM checkpoint folder with weights and no `config.json`; returns its path.
fn bare_checkpoint(home: &Path) -> Result<String> {
    let folder = home.join("edge/Bare-NVFP4");
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("model.safetensors"), vec![0_u8; 64])?;

    Ok(folder.display().to_string())
}

/// The error message of an outcome, or `succeeded`.
fn message(outcome: Outcome<()>) -> String {
    outcome.map_or_else(|error| error.to_string(), |()| "succeeded".to_string())
}

#[test]
fn the_engine_menu_says_what_each_engine_is_for_and_whether_it_is_here() -> Result<()> {
    let Some(home) = child_home() else {
        return in_child(
            "the_engine_menu_says_what_each_engine_is_for_and_whether_it_is_here",
            "",
            &[],
        )
        .map(drop);
    };
    let mut answers = Answers::new([Cancel]);

    let outcome = serve(&fake_machine(&home)?, &ask_everything(), &mut answers);
    let shown = answers.shown();

    assert!(matches!(
        outcome,
        Err(ThorError::Prompt(InquireError::OperationCanceled))
    ));
    assert!(
        shown.starts_with("Engine:\nedge-llm   NVFP4; fastest per token"),
        "{shown}"
    );
    assert!(shown.contains("llama.cpp  GGUF; the widest choice"));
    assert!(shown.contains("vllm       Hugging Face checkpoints"));
    assert!(shown.contains("not installed"));

    Ok(())
}

#[test]
fn the_model_and_context_menus_show_size_fit_and_drafter_outlook() -> Result<()> {
    let Some(home) = child_home() else {
        let (_stdout, stderr) = in_child(
            "the_model_and_context_menus_show_size_fit_and_drafter_outlook",
            "EDGE_WORKSPACE_BUILT_GIB=1\n",
            &[],
        )?;

        assert!(stderr.contains("the search failed: HF_HUB_OFFLINE is set"));
        return Ok(());
    };
    fs::create_dir_all(home.join("edge/drafters/Gemma-4-31B-IT-NVFP4"))?;
    let options = ServeOptions {
        engine: Some(Engine::EdgeLlm),
        ..ask_everything()
    };
    let mut answers = Answers::new([
        Choose(CHECKPOINT_ROW),
        Choose("16,384 tokens"),
        YesNo(false),
    ]);

    let outcome = serve(&fake_machine(&home)?, &options, &mut answers);
    let shown = answers.shown();

    assert_eq!(message(outcome), STOPPED);
    assert!(shown.contains("name, weights, longest context"), "{shown}");
    assert!(shown.contains("262,144 ctx") && shown.contains("fits at max"));
    assert!(shown.contains("search for something else\nenter a Hugging Face name"));
    assert!(shown.contains("drafter build untested at this size"));
    assert!(shown.contains("the drafter build is likely to fail"));
    assert!(shown.ends_with("another size\nGo ahead?"));

    Ok(())
}

#[test]
fn a_typed_folder_already_in_the_settings_restarts_only_the_engine() -> Result<()> {
    let Some(home) = child_home() else {
        return in_child(
            "a_typed_folder_already_in_the_settings_restarts_only_the_engine",
            "EDGE_MODEL=~/edge/Gemma-4-31B-IT-NVFP4\n",
            &[],
        )
        .map(drop);
    };
    let mut answers = Answers::new([
        Choose("edge-llm"),
        Choose("search for something else"),
        Type("no such model"),
        Choose("enter a Hugging Face name"),
        Type("~/edge/Gemma-4-31B-IT-NVFP4"),
        Choose("another size"),
        Number(16_384),
        YesNo(true),
    ]);

    let outcome = serve(&fake_machine(&home)?, &ask_everything(), &mut answers);
    let settings = fs::read_to_string(home.join("env"))?;
    let calls = fs::read_to_string(home.join("calls.log"))?;

    assert_eq!(message(outcome), "succeeded");
    assert_eq!(answers.shown().matches("on disk").count(), 1);
    assert!(settings.contains("EDGE_CONTEXT=16384\n"), "{settings}");
    assert!(calls.contains("systemctl --user restart thor-edge-llm"));
    assert!(!calls.contains("thor-tigress-agent"));

    Ok(())
}

#[test]
fn a_gguf_variant_is_downloaded_then_put_back_after_a_failed_start() -> Result<()> {
    let Some(home) = child_home() else {
        let settings = format!("SERVER=~/edge-server\nMODEL_PORT={}\n", free_port()?);
        let (stdout, _stderr) = in_child(
            "a_gguf_variant_is_downloaded_then_put_back_after_a_failed_start",
            &settings,
            &[HUB_ON, ("THOR_FAKE_FAILING", "1")],
        )?;

        assert!(stdout.contains("llama-server options: "), "{stdout}");
        assert!(stdout.contains("model card: http://127.0.0.1:"));
        assert!(stdout.contains("Restored ~/env and restarted thor-chat."));
        return Ok(());
    };
    let before = fs::read_to_string(home.join("env"))?;
    let mut answers = Answers::new([
        Choose("llama.cpp"),
        Choose("org/tiny-GGUF"),
        Choose("tiny-Q8_0"),
        Choose("4,096 tokens"),
        YesNo(true),
        YesNo(true),
    ]);

    let outcome = serve(&fake_machine(&home)?, &ask_everything(), &mut answers);
    let folder = home.join("gguf/tiny");
    let shown = answers.shown();

    assert_eq!(message(outcome), "succeeded");
    assert!(shown.contains("3 downloads") && shown.contains("too big"));
    assert_eq!(fs::read(folder.join("tiny-Q8_0.gguf"))?, vec![8_u8; 200]);
    assert!(fs::read_to_string(folder.join("config.json"))?.contains("vocab_size"));
    assert_eq!(fs::read_to_string(home.join("env"))?, before);

    Ok(())
}

#[test]
fn a_vllm_model_from_the_hub_downloads_nothing_itself() -> Result<()> {
    let Some(home) = child_home() else {
        let (stdout, _stderr) = in_child(
            "a_vllm_model_from_the_hub_downloads_nothing_itself",
            "VLLM_IMAGE=vllm:test\n",
            &[HUB_ON],
        )?;

        assert!(stdout.contains("  download   nothing\n"), "{stdout}");
        return Ok(());
    };
    let options = ServeOptions {
        engine: Some(Engine::Vllm),
        docs: false,
        ..ask_everything()
    };
    let mut answers = Answers::new([Choose("org/tiny "), Choose("16,384 tokens"), YesNo(false)]);

    let outcome = serve(&fake_machine(&home)?, &options, &mut answers);

    assert_eq!(message(outcome), STOPPED);

    Ok(())
}

#[test]
fn gguf_flags_name_the_variants_when_none_matches_or_fits() -> Result<()> {
    let Some(home) = child_home() else {
        return in_child(
            "gguf_flags_name_the_variants_when_none_matches_or_fits",
            "SERVER=~/edge-server\nMIN_FREE_GB=100000\n",
            &[HUB_ON],
        )
        .map(drop);
    };
    let machine = fake_machine(&home)?;
    let base = ServeOptions {
        engine: Some(Engine::LlamaCpp),
        model: Some("org/tiny-GGUF".to_string()),
        docs: false,
        ..ask_everything()
    };
    let unmatched = ServeOptions {
        file: Some("Q5_K_M".to_string()),
        ..base.clone()
    };
    let too_big = ServeOptions { yes: true, ..base };
    let variants = "the variants are tiny-Q4_K_M, tiny-Q8_0, tiny-BF16";

    let unmatched_outcome = serve(&machine, &unmatched, &mut Answers::new([]));
    let too_big_outcome = serve(&machine, &too_big, &mut Answers::new([]));

    assert!(message(unmatched_outcome).ends_with(variants));
    assert!(message(too_big_outcome).ends_with(variants));

    Ok(())
}

#[test]
fn a_repository_without_gguf_files_or_in_another_format_is_named() -> Result<()> {
    let Some(home) = child_home() else {
        return in_child(
            "a_repository_without_gguf_files_or_in_another_format_is_named",
            "SERVER=~/edge-server\n",
            &[HUB_ON],
        )
        .map(drop);
    };
    let machine = fake_machine(&home)?;
    let empty = ServeOptions {
        engine: Some(Engine::LlamaCpp),
        model: Some("org/empty-GGUF".to_string()),
        docs: false,
        ..ask_everything()
    };
    let other_format = ServeOptions {
        engine: Some(Engine::EdgeLlm),
        model: Some("org/tiny-GGUF".to_string()),
        docs: false,
        ..ask_everything()
    };

    let empty_outcome = serve(&machine, &empty, &mut Answers::new([]));
    let other_format_outcome = serve(&machine, &other_format, &mut Answers::new([]));

    assert_eq!(message(empty_outcome), "org/empty-GGUF has no GGUF files");
    assert_eq!(
        message(other_format_outcome),
        "org/tiny-GGUF is not in TensorRT Edge-LLM's format; its tags say --engine llama.cpp"
    );

    Ok(())
}

#[test]
fn a_checkpoint_without_a_config_asks_for_the_context_in_tokens() -> Result<()> {
    let Some(home) = child_home() else {
        let (_stdout, stderr) = in_child(
            "a_checkpoint_without_a_config_asks_for_the_context_in_tokens",
            "",
            &[],
        )?;

        assert!(stderr.contains("The memory cost of Bare-NVFP4 is unknown"));
        return Ok(());
    };
    let options = ServeOptions {
        engine: Some(Engine::EdgeLlm),
        model: Some(bare_checkpoint(&home)?),
        ..ask_everything()
    };
    let mut answers = Answers::new([Number(4096), YesNo(false)]);

    let outcome = serve(&fake_machine(&home)?, &options, &mut answers);

    assert_eq!(message(outcome), STOPPED);
    assert!(
        answers
            .shown()
            .starts_with("Context in tokens (config.json gives no maximum)")
    );

    Ok(())
}

#[test]
fn the_maximum_context_of_a_checkpoint_without_a_config_is_unknown() -> Result<()> {
    let Some(home) = child_home() else {
        return in_child(
            "the_maximum_context_of_a_checkpoint_without_a_config_is_unknown",
            "",
            &[],
        )
        .map(drop);
    };
    let options = ServeOptions {
        engine: Some(Engine::EdgeLlm),
        model: Some(bare_checkpoint(&home)?),
        context: Some(ContextChoice::Max),
        ..ask_everything()
    };

    let outcome = serve(&fake_machine(&home)?, &options, &mut Answers::new([]));

    assert_eq!(
        message(outcome),
        "Bare-NVFP4's config.json gives no maximum context; pass --context TOKENS"
    );

    Ok(())
}

#[test]
fn going_back_without_a_settings_file_says_there_is_no_backup() -> Result<()> {
    let Some(home) = child_home() else {
        let settings = format!("EDGE_PORT={}\n", free_port()?);
        let (stdout, _stderr) = in_child(
            "going_back_without_a_settings_file_says_there_is_no_backup",
            &settings,
            &[("THOR_FAKE_FAILING", "1")],
        )?;

        assert!(stdout.contains("Settings written to ~/new/env\n"));
        assert!(stdout.contains("To go back: remove the lines above from ~/new/env"));
        return Ok(());
    };
    let fake_settings = fs::read_to_string(home.join("env"))?;
    let machine = Machine {
        settings: Settings::parse(&home.join("new/env"), &home, &fake_settings),
        home: home.to_path_buf(),
        search_path: std::env::var("PATH")?,
    };
    let options = ServeOptions {
        engine: Some(Engine::EdgeLlm),
        model: Some(home.join("edge/Gemma-4-31B-IT-NVFP4").display().to_string()),
        context: Some(ContextChoice::Tokens(16_384)),
        ..ask_everything()
    };
    let mut answers = Answers::new([YesNo(true), YesNo(true)]);

    let outcome = serve(&machine, &options, &mut answers);

    assert_eq!(
        message(outcome),
        "there is no backup to restore; stop the service with: thor-tigress stop thor-edge-llm"
    );

    Ok(())
}

#[test]
fn stop_and_logs_offer_a_menu_of_the_services() -> Result<()> {
    let Some(home) = child_home() else {
        return in_child("stop_and_logs_offer_a_menu_of_the_services", "", &[]).map(drop);
    };
    let machine = fake_machine(&home)?;
    let mut declined = Answers::new([Choose("thor-edge-llm"), YesNo(false)]);
    let mut accepted = Answers::new([Choose("thor-edge-llm"), YesNo(true)]);
    let mut log = Answers::new([Choose("thor-chat")]);

    let declined_outcome = commands::stop(&machine, None, false, &mut declined);
    let calls_after_declining = fs::read_to_string(home.join("calls.log"))?;
    let accepted_outcome = commands::stop(&machine, None, false, &mut accepted);
    let log_outcome = commands::logs(None, &mut log);
    let calls = fs::read_to_string(home.join("calls.log"))?;

    assert_eq!(message(declined_outcome), STOPPED);
    assert!(!calls_after_declining.contains("systemctl --user stop"));
    assert_eq!(message(accepted_outcome), "succeeded");
    assert_eq!(message(log_outcome), "succeeded");
    assert!(declined.shown().contains("thor-vllm"));
    assert!(calls.contains("systemctl --user stop thor-edge-llm"));
    assert!(calls.contains("journalctl --user -u thor-chat -n 100 -f"));

    Ok(())
}

#[test]
fn nothing_is_downloaded_while_the_hub_is_off() -> Result<()> {
    let Some(home) = child_home() else {
        return in_child("nothing_is_downloaded_while_the_hub_is_off", "", &[]).map(drop);
    };
    let file = RepoFile {
        kind: "file".to_string(),
        path: "model.safetensors".to_string(),
        size: 10,
    };

    let outcome = Hub::from_environment(&home).download("org/tiny", &file, &home);

    assert!(
        outcome
            .err()
            .is_some_and(|error| error.to_string().starts_with("HF_HUB_OFFLINE is set;"))
    );

    Ok(())
}

#[test]
fn a_missing_journalctl_is_named() -> Result<()> {
    if child_home().is_none() {
        let thor = FakeThor::new(0, "")?;
        let only_fakes = thor.path().join("bin").display().to_string();
        fs::remove_file(thor.path().join("bin/journalctl"))?;

        return thor
            .run_test("a_missing_journalctl_is_named", &[("PATH", &only_fakes)])
            .map(drop);
    }

    let follow_outcome = service::follow_log("thor-edge-llm", 0).map(drop);
    let logs_outcome = commands::logs(Some("edge-llm".to_string()), &mut Answers::new([]));

    assert!(message(follow_outcome).starts_with("could not run journalctl: "));
    assert!(message(logs_outcome).starts_with("could not run journalctl: "));

    Ok(())
}

#[test]
fn context_rows_say_when_a_model_is_too_big_or_its_cost_is_unknown() -> Result<()> {
    let Some(home) = child_home() else {
        return in_child(
            "context_rows_say_when_a_model_is_too_big_or_its_cost_is_unknown",
            "VLLM_IMAGE=vllm:test\n",
            &[HUB_ON],
        )
        .map(drop);
    };
    let machine = fake_machine(&home)?;
    let huge = ServeOptions {
        engine: Some(Engine::Vllm),
        model: Some("org/huge".to_string()),
        docs: false,
        ..ask_everything()
    };
    let no_tree = ServeOptions {
        model: Some("org/no-tree".to_string()),
        ..huge.clone()
    };
    let mut huge_answers = Answers::new([Choose("16,384 tokens")]);
    let mut no_tree_answers = Answers::new([Choose("16,384 tokens"), YesNo(false)]);

    let huge_outcome = serve(&machine, &huge, &mut huge_answers);
    let no_tree_outcome = serve(&machine, &no_tree, &mut no_tree_answers);

    assert!(huge_answers.shown().contains("too big"));
    assert!(message(huge_outcome).starts_with("org/huge at 16,384 tokens needs about"));
    assert!(no_tree_answers.shown().contains("cost unknown"));
    assert_eq!(message(no_tree_outcome), STOPPED);

    Ok(())
}
