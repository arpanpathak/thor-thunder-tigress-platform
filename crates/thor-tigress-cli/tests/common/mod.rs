//! The fake Thor the end-to-end tests run against: fake `systemctl`, `journalctl`, `docker`
//! and `sg` on the PATH, a fake engine server and a fake Hugging Face Hub on free local
//! ports, and a temporary home folder.

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
    sync::{Arc, Mutex},
    thread,
};

use anyhow::Result;

/// The Gemma 4 31B NVFP4 config.
pub const GEMMA_CONFIG: &str = include_str!("../fixtures/gemma-4-31b-it-nvfp4.config.json");

/// A fake `systemctl --user`: it records every call, lists three services, and reports each
/// as loaded and active, holding `THOR_FAKE_MEMORY` bytes (1 GB unless set). With
/// `THOR_FAKE_FAILING` set, every `show` counts one more restart. With `THOR_FAKE_NO_UNITS`
/// set, it lists no services; `THOR_FAKE_LOAD_STATE` replaces `loaded`.
const FAKE_SYSTEMCTL: &str = r#"#!/bin/sh
echo "systemctl $*" >> "$THOR_FAKE_LOG"
case "$*" in
  *list-units*)
    if [ -n "$THOR_FAKE_NO_UNITS" ]; then exit 0; fi
    printf 'thor-chat.service loaded active running Models\nthor-edge-llm.service loaded active running Edge\nthor-vllm.service loaded active running vLLM\n' ;;
  *show*)
    restarts=$(cat "$THOR_FAKE_RESTARTS" 2>/dev/null || echo 0)
    printf 'LoadState=%s\nActiveState=active\nMemoryCurrent=%s\nMemoryMax=infinity\nNRestarts=%s\n' "${THOR_FAKE_LOAD_STATE:-loaded}" "${THOR_FAKE_MEMORY:-1000000000}" "$restarts"
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
/// stopping; the next `docker ps` still lists it and marks it stopped. With `THOR_FAKE_DOCKER_NEEDS_GROUP` set, it answers only through `sg`, as for a
/// user whose session started before they joined the `docker` group.
const FAKE_DOCKER: &str = r#"#!/bin/sh
echo "docker $*" >> "$THOR_FAKE_LOG"
if [ -n "$THOR_FAKE_DOCKER_NEEDS_GROUP" ] && [ -z "$THOR_FAKE_IN_SG" ]; then
  echo "permission denied while trying to connect to the Docker daemon socket" >&2; exit 1
fi
case "$1" in
  stats) if [ -f "$THOR_FAKE_STOPPED" ]; then echo "Error: No such container" >&2; exit 1; fi
         echo "98.5GiB / 122.8GiB" ;;
  ps) if [ -f "$THOR_FAKE_STOPPED.stopping" ]; then mv "$THOR_FAKE_STOPPED.stopping" "$THOR_FAKE_STOPPED"; echo "3f2a1b9c0d4e"
      elif [ ! -f "$THOR_FAKE_STOPPED" ]; then echo "3f2a1b9c0d4e"; fi ;;
  stop) touch "$THOR_FAKE_STOPPED.stopping"; echo "$2" ;;
esac
"#;

/// A fake `sg GROUP -c COMMAND`: it records the call and runs the command.
const FAKE_SG: &str = r#"#!/bin/sh
echo "sg $*" >> "$THOR_FAKE_LOG"
THOR_FAKE_IN_SG=1 exec sh -c "$3"
"#;

/// A home folder with fake programs, a settings file and one Edge-LLM checkpoint.
pub struct FakeThor {
    /// The temporary home folder.
    home: tempfile::TempDir,
}

impl FakeThor {
    /// Builds the home folder. `extra_settings` is appended to the settings file.
    pub fn new(port: u16, extra_settings: &str) -> Result<FakeThor> {
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

        Ok(FakeThor { home })
    }

    /// The home folder.
    pub fn path(&self) -> &Path {
        self.home.path()
    }

    /// A command for `program` with this home, the fake programs first on the PATH, and the
    /// Hub off.
    pub fn command(&self, program: &Path) -> Command {
        let path = format!("{}:/usr/bin:/bin", self.path().join("bin").display());
        let mut command = Command::new(program);
        command
            .env("HOME", self.path())
            .env("PATH", path)
            .env("HF_HUB_OFFLINE", "1")
            .env("THOR_FAKE_LOG", self.path().join("calls.log"))
            .env("THOR_FAKE_RESTARTS", self.path().join("restarts"))
            .env("THOR_FAKE_STOPPED", self.path().join("container-stopped"));

        command
    }
}

/// Writes an executable script.
pub fn write_program(path: &Path, script: &str) -> Result<()> {
    fs::write(path, script)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;

    Ok(())
}

/// A free local port with nothing listening on it.
pub fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;

    Ok(listener.local_addr()?.port())
}

/// The token the fake Hub expects; it must never appear in `thor-tigress`'s output.
pub const TEST_TOKEN: &str = "hf_test_token_never_printed";

/// A fake Hugging Face Hub on a new local port. Each request's path, without the query, is
/// looked up in `routes`; anything else gets 404. Each request is recorded as
/// `PATH token` or `PATH no-token`.
pub fn fake_hub(routes: Vec<(String, Vec<u8>)>) -> Result<(u16, Arc<Mutex<Vec<String>>>)> {
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

/// Answers every request on a new local port with an OpenAI `/v1/models` list naming
/// `model`, as an engine that has finished starting does.
pub fn fake_engine(model: &str) -> Result<u16> {
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
pub fn text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}
