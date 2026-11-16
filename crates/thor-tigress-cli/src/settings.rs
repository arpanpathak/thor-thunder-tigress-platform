//! The settings file the services read, `~/.config/thor-chat/env`: one `KEY=value` per line.
//!
//! `thor` reads it much as `thor-tigress-serve` does: the last line for a key wins, and a
//! leading `~` or `$HOME` means the home folder. Quotes differ. `thor-tigress-serve` strips
//! every quote character from both ends of a value, so `VLLM_ARGS=--x '{"a":1}'` reaches
//! vLLM as `--x '{"a":1}` and fails to parse. `thor` removes only one matching pair around
//! the whole value (`"65536"` or `'--a b'`), so it reads such a line as written.
//! A key the file does not set falls back to the default `thor-tigress-serve` uses. Writing
//! replaces a key's line in place, or appends one, and keeps every other line as it was.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::error::{Outcome, file_error};

/// The defaults `thor-tigress-serve` uses for the keys `thor` reads. Paths start at `~`.
const DEFAULTS: [(&str, &str); 14] = [
    (
        "EDGE_SERVER",
        "~/.local/share/edge-llm/venv/bin/tensorrt-edgellm-serve",
    ),
    ("EDGE_PORT", "8081"),
    ("SERVER", "~/.local/src/llama.cpp/build/bin/llama-server"),
    ("MODEL_PORT", "8079"),
    ("USERS", "4"),
    ("MIN_FREE_GB", "8"),
    ("VLLM_PORT", "8000"),
    ("VLLM_REVISION", "main"),
    ("EDGE_ROOT", "~/models/edge-llm"),
    ("GGUF_ROOT", "~/models/gguf"),
    ("HF_HUB_CACHE", "~/.cache/huggingface/hub"),
    ("VLLM_CONTAINER", "thor-vllm"),
    ("EDGE_WORKSPACE_BUILT_GIB", "48"),
    ("EDGE_WORKSPACE_FAILED_GIB", "96"),
];

/// The settings file as lines, with the home folder used to expand `~`.
#[derive(Debug)]
pub struct Settings {
    /// Where the file lives.
    pub path: PathBuf,
    /// The user's home folder.
    pub home: PathBuf,
    /// The file's lines, unchanged until [`Settings::set`] replaces one.
    lines: Vec<String>,
}

impl Settings {
    /// Reads the file. A file that does not exist yet reads as empty.
    pub fn load(path: &Path, home: &Path) -> Outcome<Settings> {
        if !path.exists() {
            return Ok(Settings::parse(path, home, ""));
        }

        let text = fs::read_to_string(path).map_err(file_error(path))?;

        Ok(Settings::parse(path, home, &text))
    }

    /// Builds the settings from the file's text.
    pub fn parse(path: &Path, home: &Path, text: &str) -> Settings {
        Settings {
            path: path.to_path_buf(),
            home: home.to_path_buf(),
            lines: text.lines().map(str::to_string).collect(),
        }
    }

    /// The value the file gives `key`, from its last line, with `~` expanded.
    pub fn get(&self, key: &str) -> Option<String> {
        self.lines
            .iter()
            .rev()
            .filter_map(|line| key_and_value(line))
            .find(|(name, _)| *name == key)
            .map(|(_, value)| self.expand(value))
    }

    /// The value the file gives `key`, else the default, else an empty string.
    pub fn value(&self, key: &str) -> String {
        let default = DEFAULTS
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| self.expand(value))
            .unwrap_or_default();

        self.get(key)
            .filter(|value| !value.is_empty())
            .unwrap_or(default)
    }

    /// The value of `key` as a path.
    pub fn path_of(&self, key: &str) -> PathBuf {
        PathBuf::from(self.value(key))
    }

    /// The value of `key` as a number, when it is one.
    pub fn number(&self, key: &str) -> Option<u64> {
        self.value(key).parse().ok()
    }

    /// Sets `key` to `value`: replaces its last line, or appends a line when there is none.
    pub fn set(&mut self, key: &str, value: &str) {
        let replacement = format!("{key}={value}");
        let last_line = self
            .lines
            .iter_mut()
            .rev()
            .find(|line| key_and_value(line).is_some_and(|(name, _)| name == key));

        match last_line {
            Some(line) => *line = replacement,
            None => self.lines.push(replacement),
        }
    }

    /// The file's text as it would be written.
    pub fn text(&self) -> String {
        let mut text = self.lines.join("\n");
        text.push('\n');

        text
    }

    /// Copies the file as it is on disk to `env.backup-SECONDS`, then writes the new text.
    /// Returns the backup's path, or `None` when there was no file to back up.
    pub fn save_with_backup(&self, seconds_since_epoch: u64) -> Outcome<Option<PathBuf>> {
        let backup = self.path.with_file_name(format!(
            "{}.backup-{seconds_since_epoch}",
            file_name(&self.path)
        ));
        let had_file = self.path.exists();

        if had_file {
            fs::copy(&self.path, &backup).map_err(file_error(&backup))?;
        }

        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder).map_err(file_error(folder))?;
        }

        fs::write(&self.path, self.text()).map_err(file_error(&self.path))?;

        Ok(had_file.then_some(backup))
    }

    /// Replaces a leading `~` or `$HOME` with the home folder.
    fn expand(&self, value: &str) -> String {
        let home = self.home.display().to_string();
        let rest = value
            .strip_prefix('~')
            .or_else(|| value.strip_prefix("$HOME"));

        match rest {
            Some(rest) => format!("{home}{rest}"),
            None => value.to_string(),
        }
    }
}

/// The key and the unquoted value of a `KEY=value` line; `None` for comments and other lines.
fn key_and_value(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.trim().split_once('=')?;
    let key = key.trim();

    if key.starts_with('#') {
        return None;
    }

    Some((key, without_surrounding_quotes(value.trim())))
}

/// A value without one pair of matching quotes around the whole of it. A quote at only one
/// end, as in `--x '{"a":1}'`, belongs to the value and stays.
pub fn without_surrounding_quotes(value: &str) -> &str {
    let double = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'));
    let single = value
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''));

    double.or(single).unwrap_or(value)
}

/// What `thor-tigress-serve` reads for a value: every quote character stripped from both
/// ends. A value `thor` writes must come through this unchanged.
pub fn as_thor_tigress_serve_reads(value: &str) -> &str {
    value.trim().trim_matches(['"', '\''])
}

/// The last part of a path, as text.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}
