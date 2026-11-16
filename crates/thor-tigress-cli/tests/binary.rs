//! The `thor` binary end to end, against a fake `systemctl`, `journalctl`, `docker` and `sg`
//! on the PATH,
//! a fake engine server on a free local port, and `HF_HUB_OFFLINE=1`. Nothing here touches
//! the network, a real service or a real model.

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{Arc, Mutex},
    thread,
};

use anyhow::{Result, anyhow};

/// The Gemma 4 31B NVFP4 config.
const GEMMA_CONFIG: &str = include_str!("fixtures/gemma-4-31b-it-nvfp4.config.json");

/// A fake `systemctl --user`: it records every call, lists three services, and reports each
/// as loaded and active, holding `THOR_FAKE_MEMORY` bytes (1 GB unless set). With
/// `THOR_FAKE_FAILING` set, every `show` counts one more restart.
const FAKE_SYSTEMCTL: &str = r#"#!/bin/sh
echo "systemctl $*" >> "$THOR_FAKE_LOG"
case "$*" in
  *list-units*)
    printf 'thor-chat.service loaded active running Models\nthor-edge-llm.service loaded active running Edge\nthor-vllm.service loaded active running vLLM\n' ;;
  *show*)
    restarts=$(cat "$THOR_FAKE_RESTARTS" 2>/dev/null || echo 0)
    printf 'LoadState=loaded\nActiveState=active\nMemoryCurrent=%s\nMemoryMax=infinity\nNRestarts=%s\n' "${THOR_FAKE_MEMORY:-1000000000}" "$restarts"
    if [ -n "$THOR_FAKE_FAILING" ]; then echo $((restarts + 1)) > "$THOR_FAKE_RESTARTS"; fi ;;
esac
"#;

/// A fake `journalctl`: following prints one line and waits; reading prints a TensorRT
/// out-of-memory error.
const FAKE_JOURNALCTL: &str = r#"#!/bin/sh
echo "journalctl $*" >> "$THOR_FAKE_LOG"
case "$*" in
  *" -f "*) echo "Building the TensorRT engine"; exec sleep 30 ;;
  *) echo "[TensorRT] ERROR: [virtualMemoryBuffer.cpp::resizePhysical::154] Error Code 2: OutOfMemory (ran out of memory)"
     echo "loading weights" ;;
esac
"#;

/// A fake `docker`: the container `thor-vllm` uses 98.5 GiB until `docker stop` marks it
/// stopped. With `THOR_FAKE_DOCKER_NEEDS_GROUP` set, it answers only through `sg`, as for a
/// user whose session started before they joined the `docker` group.
const FAKE_DOCKER: &str = r#"#!/bin/sh
echo "docker $*" >> "$THOR_FAKE_LOG"
if [ -n "$THOR_FAKE_DOCKER_NEEDS_GROUP" ] && [ -z "$THOR_FAKE_IN_SG" ]; then
  echo "permission denied while trying to connect to the Docker daemon socket" >&2; exit 1
fi
case "$1" in
  stats) if [ -f "$THOR_FAKE_STOPPED" ]; then echo "Error: No such container" >&2; exit 1; fi
         echo "98.5GiB / 122.8GiB" ;;
  ps) if [ ! -f "$THOR_FAKE_STOPPED" ]; then echo "3f2a1b9c0d4e"; fi ;;
  stop) touch "$THOR_FAKE_STOPPED"; echo "$2" ;;
esac
"#;

/// A fake `sg GROUP -c COMMAND`: it records the call and runs the command.
const FAKE_SG: &str = r#"#!/bin/sh
echo "sg $*" >> "$THOR_FAKE_LOG"
THOR_FAKE_IN_SG=1 exec sh -c "$3"
"#;

/// A home folder with fake programs, a settings file and one Edge-LLM checkpoint.
struct FakeThor {
    /// The temporary home folder.
    home: tempfile::TempDir,
    /// The engine port written in the settings.
    port: u16,
}

impl FakeThor {
    /// Builds the home folder. `extra_settings` is appended to the settings file.
    fn new(port: u16, extra_settings: &str) -> Result<FakeThor> {
        let home = tempfile::tempdir()?;
        let bin = home.path().join("bin");
        let checkpoint = home.path().join("edge/Gemma-4-31B-IT-NVFP4");
        fs::create_dir_all(&bin)?;
        fs::create_dir_all(&checkpoint)?;

        write_program(&bin.join("systemctl"), FAKE_SYSTEMCTL)?;
        write_program(&bin.join("journalctl"), FAKE_JOURNALCTL)?;
        write_program(&bin.join("docker"), FAKE_DOCKER)?;
        write_program(&bin.join("sg"), FAKE_SG)?;
        fs::write(checkpoint.join("config.json"), GEMMA_CONFIG)?;
        fs::write(checkpoint.join("model.safetensors"), vec![0_u8; 64])?;

        write_program(&home.path().join("edge-server"), "#!/bin/sh\n")?;
        let settings = format!(
            "EDGE_MODEL=~/edge/Old-NVFP4\nEDGE_SERVER=~/edge-server\nEDGE_PORT={port}\nEDGE_ROOT=~/edge\nGGUF_ROOT=~/gguf\nMIN_FREE_GB=0\n{extra_settings}"
        );

        fs::write(home.path().join("env"), settings)?;

        Ok(FakeThor { home, port })
    }

    /// The home folder.
    fn path(&self) -> &Path {
        self.home.path()
    }

    /// The checkpoint folder.
    fn checkpoint(&self) -> PathBuf {
        self.path().join("edge/Gemma-4-31B-IT-NVFP4")
    }

    /// Runs `thor` with this home, the fake programs first on the PATH, and the Hub off.
    fn run(&self, arguments: &[&str], extra_environment: &[(&str, &str)]) -> Result<Output> {
        let path = format!("{}:/usr/bin:/bin", self.path().join("bin").display());
        let settings = self.path().join("env");
        let mut command = Command::new(env!("CARGO_BIN_EXE_thor"));
        command
            .args(arguments)
            .args(["--settings", &settings.display().to_string()])
            .env("HOME", self.path())
            .env("PATH", path)
            .env("HF_HUB_OFFLINE", "1")
            .env("THOR_FAKE_LOG", self.path().join("calls.log"))
            .env("THOR_FAKE_RESTARTS", self.path().join("restarts"))
            .env("THOR_FAKE_STOPPED", self.path().join("container-stopped"));

        for (name, value) in extra_environment {
            command.env(name, value);
        }

        Ok(command.output()?)
    }

    /// Runs `thor` against the fake Hub on `hub_port`, with the test token.
    fn run_with_hub(&self, hub_port: u16, arguments: &[&str]) -> Result<Output> {
        let endpoint = format!("http://127.0.0.1:{hub_port}");
        let environment = [
            ("HF_HUB_OFFLINE", "0"),
            ("HF_ENDPOINT", endpoint.as_str()),
            ("HF_TOKEN", TEST_TOKEN),
        ];

        self.run(arguments, &environment)
    }

    /// Runs `thor serve --yes` on the checkpoint with Edge-LLM and the given context.
    fn serve_checkpoint(
        &self,
        context: &str,
        extra_environment: &[(&str, &str)],
    ) -> Result<Output> {
        let checkpoint = self.checkpoint().display().to_string();
        let arguments = [
            "serve",
            "--engine",
            "edge-llm",
            "--model",
            &checkpoint,
            "--context",
            context,
            "--yes",
            "--wait",
            "30",
        ];

        self.run(&arguments, extra_environment)
    }

    /// The text of the settings backup `thor serve` made.
    fn backup(&self) -> Result<String> {
        let backup = fs::read_dir(self.path())?
            .flatten()
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("env.backup-")
            })
            .ok_or_else(|| anyhow!("no backup"))?;

        Ok(fs::read_to_string(backup.path())?)
    }

    /// Every call the fake programs recorded.
    fn calls(&self) -> String {
        fs::read_to_string(self.path().join("calls.log")).unwrap_or_default()
    }

    /// The settings file now.
    fn settings(&self) -> Result<String> {
        Ok(fs::read_to_string(self.path().join("env"))?)
    }
}

/// Writes an executable script.
fn write_program(path: &Path, script: &str) -> Result<()> {
    fs::write(path, script)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;

    Ok(())
}

/// A free local port with nothing listening on it.
fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;

    Ok(listener.local_addr()?.port())
}

/// The token the fake Hub expects; it must never appear in `thor`'s output.
const TEST_TOKEN: &str = "hf_test_token_never_printed";

/// A fake Hugging Face Hub on a new local port. Each request's path, without the query, is
/// looked up in `routes`; anything else gets 404. Each request is recorded as
/// `PATH token` or `PATH no-token`.
fn fake_hub(routes: Vec<(String, Vec<u8>)>) -> Result<(u16, Arc<Mutex<Vec<String>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);

    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let record = answer_hub_request(&mut stream, &routes);

            if let Ok(mut list) = recorded.lock() {
                list.push(record);
            }
        }
    });

    Ok((port, requests))
}

/// Answers one request to the fake Hub and returns what to record about it.
fn answer_hub_request(stream: &mut TcpStream, routes: &[(String, Vec<u8>)]) -> String {
    let mut buffer = [0_u8; 8192];
    let size = stream.read(&mut buffer).unwrap_or(0);
    let request = String::from_utf8_lossy(&buffer[..size]).to_string();
    let target = request.split_whitespace().nth(1).unwrap_or_default();
    let path = target.split('?').next().unwrap_or_default().to_string();
    let token = if request.contains(&format!("Bearer {TEST_TOKEN}")) {
        "token"
    } else {
        "no-token"
    };
    let body = routes
        .iter()
        .find(|(route, _body)| *route == path)
        .map(|(_route, body)| body);
    let status = if body.is_some() {
        "200 OK"
    } else {
        "404 Not Found"
    };
    let body: &[u8] = body.map_or(&[], Vec::as_slice);
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );

    stream.write_all(head.as_bytes()).ok();
    stream.write_all(body).ok();

    format!("{path} {token}")
}

/// The fake Hub's answers for one Edge-LLM checkpoint with a config and 1000 bytes of
/// weights.
fn gemma_hub_routes() -> Vec<(String, Vec<u8>)> {
    let repo = "/nvidia/Gemma-4-31B-IT-NVFP4";
    let model =
        r#"{"id":"nvidia/Gemma-4-31B-IT-NVFP4","downloads":5,"tags":["modelopt","safetensors"]}"#;
    let tree = format!(
        r#"[{{"type":"file","path":"config.json","size":{}}},{{"type":"file","path":"model.safetensors","size":1000}},{{"type":"directory","path":"extra","size":0}}]"#,
        GEMMA_CONFIG.len()
    );

    vec![
        (format!("/api/models{repo}"), model.as_bytes().to_vec()),
        (format!("/api/models{repo}/tree/main"), tree.into_bytes()),
        (
            format!("{repo}/resolve/main/config.json"),
            GEMMA_CONFIG.as_bytes().to_vec(),
        ),
        (
            format!("{repo}/resolve/main/model.safetensors"),
            vec![7_u8; 1000],
        ),
    ]
}

/// Answers every request on a new local port with an OpenAI `/v1/models` list naming
/// `model`, as an engine that has finished starting does.
fn fake_engine(model: &str) -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let body = format!(r#"{{"object":"list","data":[{{"id":"{model}","object":"model"}}]}}"#);

    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0_u8; 4096];
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );

            if stream.read(&mut request).is_ok() {
                stream.write_all(reply.as_bytes()).ok();
            }
        }
    });

    Ok(port)
}

/// Standard output and standard error as text.
fn text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn serve_writes_the_settings_restarts_and_waits_for_the_answer() -> Result<()> {
    let port = fake_engine("Gemma-4-31B-IT-NVFP4")?;
    let thor = FakeThor::new(port, "")?;
    let checkpoint = thor.checkpoint().display().to_string();

    let output = thor.serve_checkpoint("16384", &[])?;
    let (stdout, stderr) = text(&output);
    let settings = thor.settings()?;

    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(settings.contains(&format!("EDGE_MODEL={checkpoint}\n")));
    assert!(settings.contains("EDGE_CONTEXT=16384\nEDGE_PROMPT=8192\n"));
    assert!(thor.backup()?.contains("EDGE_MODEL=~/edge/Old-NVFP4"));
    assert!(
        thor.calls()
            .contains("systemctl --user restart thor-edge-llm")
    );
    assert!(
        thor.calls()
            .contains("systemctl --user restart thor-tigress-agent")
    );
    assert!(stdout.contains(&format!(
        "Ready: Gemma-4-31B-IT-NVFP4 answers on 127.0.0.1:{}",
        thor.port
    )));

    Ok(())
}

#[test]
fn a_failed_start_prints_the_errors_and_how_to_go_back() -> Result<()> {
    let thor = FakeThor::new(free_port()?, "")?;

    let output = thor.serve_checkpoint("16384", &[("THOR_FAKE_FAILING", "1")])?;
    let (stdout, stderr) = text(&output);

    assert!(!output.status.success());
    assert!(stdout.contains("thor-edge-llm failed."), "{stdout}");
    assert!(stdout.contains("Error Code 2: OutOfMemory (ran out of memory)"));
    assert!(!stdout.contains("  loading weights"));
    assert!(stdout.contains("To go back:\n  cp "));
    assert!(stdout.contains("systemctl --user restart thor-edge-llm"));
    assert!(stderr.contains("Gemma-4-31B-IT-NVFP4 is not serving"));

    Ok(())
}

#[test]
fn a_model_that_does_not_fit_changes_nothing() -> Result<()> {
    let thor = FakeThor::new(free_port()?, "MIN_FREE_GB=100000\n")?;
    let before = thor.settings()?;

    let output = thor.serve_checkpoint("max", &[])?;
    let (_stdout, stderr) = text(&output);

    assert!(!output.status.success());
    assert!(
        stderr.contains("Gemma-4-31B-IT-NVFP4 at 262,144 tokens needs about 11.2 GB;"),
        "{stderr}"
    );
    assert!(stderr.contains("To make room, stop thor-chat (1.0 GB): thor stop thor-chat"));
    assert_eq!(thor.settings()?, before);
    assert!(!thor.calls().contains("restart"));

    Ok(())
}

#[test]
fn list_shows_services_and_models_on_disk() -> Result<()> {
    let thor = FakeThor::new(free_port()?, "")?;
    let checkpoint = thor.checkpoint().display().to_string();
    fs::write(
        thor.path().join("env"),
        format!("EDGE_MODEL={checkpoint}\nEDGE_ROOT=~/edge\n"),
    )?;

    let output = thor.run(&["list"], &[])?;
    let (stdout, _stderr) = text(&output);

    assert!(output.status.success());
    assert!(stdout.contains("thor-chat"));
    assert!(stdout.contains("thor-edge-llm"));
    assert!(stdout.contains("edge-llm   Gemma-4-31B-IT-NVFP4"));
    assert!(stdout.contains("262,144 ctx"));
    assert!(stdout.contains("in the settings (EDGE_MODEL)"));

    Ok(())
}

#[test]
fn stop_asks_systemd_to_stop_the_named_service() -> Result<()> {
    let thor = FakeThor::new(free_port()?, "")?;

    let output = thor.run(&["stop", "edge-llm", "--yes"], &[])?;

    assert!(output.status.success());
    assert!(thor.calls().contains("systemctl --user stop thor-edge-llm"));

    Ok(())
}

#[test]
fn engines_detects_what_is_installed() -> Result<()> {
    let thor = FakeThor::new(free_port()?, "")?;
    let server = thor.path().join("tensorrt-edgellm-serve");
    let docker = thor.path().join("bin/docker");
    fs::write(&server, "")?;
    write_program(&docker, "#!/bin/sh\n")?;
    fs::write(
        thor.path().join("env"),
        format!(
            "EDGE_SERVER={}\nSERVER=~/missing/llama-server\nVLLM_IMAGE=vllm-image:test\n",
            server.display()
        ),
    )?;

    let output = thor.run(&["engines"], &[])?;
    let (stdout, _stderr) = text(&output);

    assert!(output.status.success());
    assert!(stdout.contains(&format!(
        "TensorRT Edge-LLM (--engine edge-llm)\n  installed: {}",
        server.display()
    )));
    assert!(stdout.contains("llama.cpp (--engine llama.cpp)\n  not installed:"));
    assert!(stdout.contains(&format!(
        "vLLM (--engine vllm)\n  installed: {} with image vllm-image:test",
        docker.display()
    )));
    assert!(stdout.contains(
        "example: thor serve --engine vllm --model ORGANIZATION/REPOSITORY --context max"
    ));

    Ok(())
}

#[test]
fn a_hub_name_is_not_asked_for_when_offline() -> Result<()> {
    let thor = FakeThor::new(free_port()?, "")?;

    let output = thor.run(
        &[
            "serve",
            "--engine",
            "edge-llm",
            "--model",
            "org/not-on-disk",
            "--yes",
        ],
        &[],
    )?;
    let (_stdout, stderr) = text(&output);

    assert!(!output.status.success());
    assert!(stderr.contains("HF_HUB_OFFLINE is set"), "{stderr}");

    Ok(())
}

/// `thor serve` for the fake Hub's checkpoint, without fetching documentation.
const HUB_SERVE: [&str; 11] = [
    "serve",
    "--engine",
    "edge-llm",
    "--model",
    "nvidia/Gemma-4-31B-IT-NVFP4",
    "--context",
    "16384",
    "--yes",
    "--no-docs",
    "--wait",
    "30",
];

#[test]
fn a_hub_model_is_downloaded_with_the_token_then_served() -> Result<()> {
    let thor = FakeThor::new(fake_engine("Gemma-4-31B-IT-NVFP4")?, "")?;
    let (hub_port, requests) = fake_hub(gemma_hub_routes())?;
    let target = thor.path().join("edge/Gemma-4-31B-IT-NVFP4");
    fs::create_dir_all(&target)?;
    fs::write(target.join("model.safetensors.part"), b"stale bytes")?;

    let output = thor.run_with_hub(hub_port, &HUB_SERVE)?;
    let (stdout, stderr) = text(&output);
    let requests = requests
        .lock()
        .map_err(|error| anyhow!("{error}"))?
        .to_vec();

    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        fs::read(target.join("model.safetensors"))?,
        vec![7_u8; 1000]
    );
    assert!(!target.join("model.safetensors.part").exists());
    assert!(
        thor.settings()?
            .contains(&format!("EDGE_MODEL={}\n", target.display()))
    );
    assert!(
        requests.iter().all(|request| request.ends_with(" token")),
        "{requests:?}"
    );
    assert!(!stdout.contains(TEST_TOKEN) && !stderr.contains(TEST_TOKEN));

    Ok(())
}

#[test]
fn an_engine_that_is_not_installed_is_reported() -> Result<()> {
    let thor = FakeThor::new(free_port()?, "")?;
    fs::write(thor.path().join("env"), "SERVER=~/missing/llama-server\n")?;

    let output = thor.run(&["serve", "--engine", "llama.cpp", "--yes"], &[])?;
    let (_stdout, stderr) = text(&output);

    assert!(!output.status.success());
    assert!(
        stderr.contains("llama.cpp is not installed here:"),
        "{stderr}"
    );
    assert!(stderr.contains("missing/llama-server not found. See thor engines."));

    Ok(())
}

#[test]
fn list_counts_the_vllm_container_memory() -> Result<()> {
    let thor = FakeThor::new(free_port()?, "")?;

    let output = thor.run(&["list"], &[])?;
    let (stdout, _stderr) = text(&output);
    let vllm_row = stdout
        .lines()
        .find(|line| line.contains("thor-vllm"))
        .ok_or_else(|| anyhow!("no thor-vllm row: {stdout}"))?;

    assert!(
        vllm_row.contains("105.8 GB (Docker container)"),
        "{vllm_row}"
    );
    assert!(
        thor.calls()
            .contains("docker stats --no-stream --format {{.MemUsage}} thor-vllm")
    );

    Ok(())
}

#[test]
fn stopping_vllm_stops_its_container_without_a_terminal() -> Result<()> {
    let thor = FakeThor::new(free_port()?, "")?;

    let output = thor.run(&["stop", "thor-vllm"], &[])?;
    let (stdout, stderr) = text(&output);

    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(thor.calls().contains("systemctl --user stop thor-vllm"));
    assert!(thor.calls().contains("docker stop thor-vllm"));
    assert!(stdout.contains("Docker container thor-vllm stopped."));
    assert!(stdout.contains("Stopped thor-vllm. Freed "));

    Ok(())
}

#[test]
fn docker_runs_through_sg_when_the_session_lacks_the_group() -> Result<()> {
    let thor = FakeThor::new(free_port()?, "")?;

    let output = thor.run(&["list"], &[("THOR_FAKE_DOCKER_NEEDS_GROUP", "1")])?;
    let (stdout, _stderr) = text(&output);

    assert!(stdout.contains("105.8 GB (Docker container)"), "{stdout}");
    assert!(thor.calls().contains(
        "sg docker -c 'docker' 'stats' '--no-stream' '--format' '{{.MemUsage}}' 'thor-vllm'"
    ));

    Ok(())
}

#[test]
fn a_drafter_build_with_a_long_input_gets_a_note() -> Result<()> {
    let thor = FakeThor::new(fake_engine("Gemma-4-31B-IT-NVFP4")?, "")?;
    fs::create_dir_all(thor.path().join("edge/drafters/Gemma-4-31B-IT-NVFP4"))?;

    let output = thor.serve_checkpoint("262144", &[("THOR_FAKE_MEMORY", "100000000000")])?;
    let (stdout, stderr) = text(&output);

    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains(
        "(max input 131,072 / 2) × vocabulary 262,144 × 6 bytes = 96.0 GiB for the LM head."
    ));
    assert!(stdout.contains("Builds needing 96.0 GiB or more failed on this Thor"));
    assert!(stdout.contains("65,536 tokens (--context 131072 keeps it)."));

    Ok(())
}
