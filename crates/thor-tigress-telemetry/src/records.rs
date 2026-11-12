//! The agent's request log: one JSON line per model request, read as it grows.
//!
//! [`Tail`] remembers how far it has read, so each call reads only the new
//! lines. A line still being written (no newline yet) is left for the next
//! call. A log that got shorter was rotated or emptied, and is read again from
//! the start. Each line goes on as it was written, with OpenObserve's time
//! field `_timestamp` (microseconds) added from the agent's `at_ms`, so a field
//! the agent adds later reaches the dashboard without a change here.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};

use serde_json::{Map, Value};

use crate::error::Outcome;

/// How far the log has been read.
#[derive(Debug)]
pub struct Tail {
    path: PathBuf,
    offset: u64,
}

impl Tail {
    /// A reader that starts at the beginning of the log at `path`.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Tail { path, offset: 0 }
    }

    /// The complete lines written since the last call, as JSON objects. A
    /// line that is not an object is skipped. A missing log has no lines yet.
    ///
    /// # Errors
    ///
    /// The I/O error when the log exists but cannot be read.
    pub fn read_new(&mut self) -> Outcome<Vec<Value>> {
        let Some(file) = open_if_present(&self.path)? else {
            return Ok(Vec::new());
        };

        let text = self.unread(file)?;
        let complete = text.rfind('\n').map_or(0, |end| end + 1);

        self.offset += complete as u64;

        Ok(text[..complete].lines().filter_map(parse).collect())
    }

    /// Everything after the offset, from the start when the file got shorter.
    fn unread(&mut self, mut file: File) -> Outcome<String> {
        let length = file.metadata()?.len();
        let mut text = String::new();

        self.offset = start(self.offset, length);
        file.seek(SeekFrom::Start(self.offset))?;
        file.read_to_string(&mut text)?;

        Ok(text)
    }
}

/// Where to start reading a file `length` bytes long: at `offset`, or at the
/// start when the file is now shorter than that.
fn start(offset: u64, length: u64) -> u64 {
    if length < offset {
        return 0;
    }

    offset
}

fn open_if_present(path: &PathBuf) -> Outcome<Option<File>> {
    match File::open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn parse(line: &str) -> Option<Value> {
    let mut record: Map<String, Value> = serde_json::from_str(line).ok()?;
    let at_ms = record.get("at_ms").and_then(Value::as_u64).unwrap_or(0);

    record.insert("_timestamp".to_string(), Value::from(at_ms * 1000));

    Some(Value::Object(record))
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use anyhow::Result;

    use super::*;

    fn line(at_ms: u64, who: &str) -> String {
        format!("{{\"at_ms\":{at_ms},\"who\":\"{who}\",\"status\":200}}\n")
    }

    fn names(records: &[Value]) -> Vec<&str> {
        records
            .iter()
            .filter_map(|record| record["who"].as_str())
            .collect()
    }

    #[test]
    fn reads_only_new_complete_lines() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("requests.jsonl");
        let mut tail = Tail::new(path.clone());
        let missing = tail.read_new()?;

        fs::write(&path, line(1, "ada") + "not json\n[1]\n" + "{\"at_ms\":2")?;
        let first = tail.read_new()?;

        fs::OpenOptions::new()
            .append(true)
            .open(&path)?
            .write_all(b",\"who\":\"grace\"}\n")?;
        let second = tail.read_new()?;
        let third = tail.read_new()?;

        assert!(missing.is_empty());
        assert_eq!(names(&first), ["ada"]);
        assert_eq!(first[0]["_timestamp"], 1_000);
        assert_eq!(names(&second), ["grace"]);
        assert!(third.is_empty());

        Ok(())
    }

    #[test]
    fn starts_again_when_the_log_was_rotated() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("requests.jsonl");
        let mut tail = Tail::new(path.clone());

        fs::write(&path, line(1, "ada") + &line(2, "ada"))?;
        let before = tail.read_new()?;

        fs::write(&path, line(3, "grace"))?;
        let after = tail.read_new()?;

        assert_eq!(names(&before), ["ada", "ada"]);
        assert_eq!(names(&after), ["grace"]);

        Ok(())
    }

    #[test]
    fn a_log_that_cannot_be_opened_or_read_is_an_error() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let folder = tempfile::tempdir()?;
        let locked = folder.path().join("requests.jsonl");

        fs::write(&locked, line(1, "ada"))?;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;

        assert!(Tail::new(locked).read_new().is_err());
        assert!(
            Tail::new(PathBuf::from("/proc/self/mem"))
                .read_new()
                .is_err()
        );

        Ok(())
    }
}
