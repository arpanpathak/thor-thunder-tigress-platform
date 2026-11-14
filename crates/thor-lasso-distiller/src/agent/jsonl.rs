//! JSON Lines files: one JSON value per line.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use serde::{Serialize, de::DeserializeOwned};

use crate::error::DistillError;

/// Parses each non-blank line of `path` as a `T`.
///
/// # Errors
///
/// [`DistillError::Io`] for a file that can't be read, and
/// [`DistillError::Json`] for a line that is not a `T`.
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, DistillError> {
    let text = fs::read_to_string(path).map_err(DistillError::io(path))?;

    parse(path, &text)
}

/// Parses `path` as [`read`] does, and returns an empty list when the file
/// does not exist.
///
/// # Errors
///
/// As [`read`], for a file that exists.
pub fn read_if_present<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, DistillError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    read(path)
}

/// Parses each non-blank line of `text`, the contents of `path`, as a `T`.
fn parse<T: DeserializeOwned>(path: &Path, text: &str) -> Result<Vec<T>, DistillError> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(DistillError::json(path, index + 1))
        })
        .collect()
}

/// Writes `records` to `path`, one per line, replacing the file's contents.
///
/// # Errors
///
/// [`DistillError::Io`] when the file can't be written.
pub fn write<T: Serialize>(path: &Path, records: &[T]) -> Result<(), DistillError> {
    let lines: Vec<String> = records.iter().map(line).collect::<Result<_, _>>()?;
    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();

    fs::write(path, text).map_err(DistillError::io(path))
}

/// Appends `record` as one line to `path`, creating the file when it does
/// not exist.
///
/// # Errors
///
/// [`DistillError::Io`] when the file can't be written.
pub fn append<T: Serialize>(path: &Path, record: &T) -> Result<(), DistillError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(DistillError::io(path))?;

    writeln!(file, "{}", line(record)?).map_err(DistillError::io(path))
}

/// Serializes `record` as one line of JSON.
fn line<T: Serialize>(record: &T) -> Result<String, DistillError> {
    serde_json::to_string(record).map_err(DistillError::json("record", 1))
}
