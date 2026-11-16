//! This machine: its settings file, its memory, and the services running on it.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    engine::Engine,
    error::{Outcome, ThorError},
    memory::{self, Budget, GIGABYTE},
    docker,
    service::{self, UnitState},
    settings::Settings,
};

/// The settings file, relative to the home folder.
pub const SETTINGS_FILE: &str = ".config/thor-chat/env";

/// Where the kernel reports memory.
const MEMINFO: &str = "/proc/meminfo";

/// What `thor-tigress` knows about the machine it runs on.
#[derive(Debug)]
pub struct Machine {
    /// The settings file.
    pub settings: Settings,
    /// The home folder.
    pub home: PathBuf,
    /// The PATH, for finding programs.
    pub search_path: String,
}

impl Machine {
    /// Reads `$HOME`, `$PATH` and the settings file, `--settings` or the default.
    pub fn load(settings_path: Option<PathBuf>) -> Outcome<Machine> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| ThorError::Condition("HOME is not set".to_string()))?;
        let settings_path = settings_path.unwrap_or_else(|| home.join(SETTINGS_FILE));

        Ok(Machine {
            settings: Settings::load(&settings_path, &home)?,
            search_path: std::env::var("PATH").unwrap_or_default(),
            home,
        })
    }

    /// The memory budget for a model on `engine`: what is available now, plus what the
    /// engine's service holds and gives back when it restarts, minus `MIN_FREE_GB`, and no
    /// more than the service's own `MemoryMax`.
    pub fn budget(&self, engine: Engine) -> Budget {
        let meminfo = self.meminfo();
        let state = service::unit_state(engine.service())
            .ok()
            .map(|state| self.with_container_memory(state));
        let holding = state.as_ref().filter(|state| state.is_holding_memory());
        let keep_free_gigabytes = self.settings.number("MIN_FREE_GB").unwrap_or(0);

        Budget {
            available: memory::available_bytes(&meminfo).unwrap_or(0),
            freed_by_restart: holding.and_then(UnitState::held_memory).unwrap_or(0),
            keep_free: (keep_free_gigabytes as f64 * GIGABYTE) as u64,
            service_limit: state.and_then(|state| state.memory_max),
        }
    }

    /// The models folder of an engine.
    pub fn models_root(&self, engine: Engine) -> PathBuf {
        self.settings.path_of(engine.models_key())
    }

    /// The `/proc/meminfo` text, empty when it cannot be read.
    pub fn meminfo(&self) -> String {
        fs::read_to_string(MEMINFO).unwrap_or_default()
    }

    /// `MemAvailable` now, in bytes; 0 when it cannot be read.
    pub fn available(&self) -> u64 {
        memory::available_bytes(&self.meminfo()).unwrap_or(0)
    }

    /// Every `thor-*` service with its state and, for vLLM, its container's memory; empty
    /// when systemd cannot be asked.
    pub fn thor_services(&self) -> Vec<UnitState> {
        service::thor_units()
            .unwrap_or_default()
            .iter()
            .filter_map(|name| service::unit_state(name).ok())
            .map(|state| self.with_container_memory(state))
            .collect()
    }

    /// The services holding memory other than `except`, largest first.
    pub fn services_holding_memory_except(&self, except: &str) -> Vec<UnitState> {
        let mut holding: Vec<UnitState> = self
            .thor_services()
            .into_iter()
            .filter(|state| state.is_holding_memory() && state.name != except)
            .collect();
        holding.sort_by_key(|state| std::cmp::Reverse(state.held_memory()));

        holding
    }

    /// Adds the Docker container's memory to the vLLM service's state (`VLLM_CONTAINER`).
    pub fn with_container_memory(&self, mut state: UnitState) -> UnitState {
        if state.name == Engine::Vllm.service() {
            state.container = docker::container_memory(&self.settings.value("VLLM_CONTAINER"));
        }

        state
    }
}

/// Seconds since 1970, for backup names and log times.
pub fn seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// A path for people to read, with the home folder written as `~`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_outside_home) => path.display().to_string(),
    }
}

/// A path written with `~` expanded to the home folder.
pub fn expand_home(text: &str, home: &Path) -> PathBuf {
    match text.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(text),
    }
}
