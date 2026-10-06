//! Reading and writing files that hold one JSON value per line.

use std::{
    fs,
    io::ErrorKind,
    path::Path,
};

use serde::{Serialize, de::DeserializeOwned};

use crate::error::{Outcome, ReviewError};

/// Every line of `path` as a `T`, skipping blank lines. A missing file reads
/// as no lines, so the tool works before the first build or the first flag.
///
/// # Errors
///
/// `ReviewError::Io` when the file can't be read, `ReviewError::Json` naming
/// the first line that isn't a `T`.
pub fn read_lines<T: DeserializeOwned>(path: &Path) -> Outcome<Vec<T>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(ReviewError::io(path)(error)),
    };
    text.lines()
        .zip(1..)
        .filter(|(line, _)| !line.trim().is_empty())
        .map(|(line, number)| serde_json::from_str(line).map_err(ReviewError::json(path, number)))
        .collect()
}

/// Writes `items` to `path`, one per line, through a temporary file so a
/// crash can't leave the file half-written. Creates the folder if needed.
///
/// # Errors
///
/// `ReviewError::Io` when the folder or file can't be written.
pub fn write_lines<T: Serialize>(path: &Path, items: &[T]) -> Outcome {
    let mut text = String::new();
    for item in items {
        let line = serde_json::to_string(item).map_err(|error| ReviewError::BadRequest(error.to_string()))?;
        text.push_str(&line);
        text.push('\n');
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(ReviewError::io(parent))?;
    }
    let temporary = path.with_extension("jsonl.tmp");
    fs::write(&temporary, text).map_err(ReviewError::io(&temporary))?;
    fs::rename(&temporary, path).map_err(ReviewError::io(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Line {
        id: String,
    }

    #[test]
    fn writes_and_reads_back_skipping_blank_lines() -> Outcome {
        let folder = TempDir::new()?;
        let path = folder.path().join("nested/lines.jsonl");
        write_lines(&path, &[Line { id: "a".to_string() }, Line { id: "b".to_string() }])?;
        fs::write(&path, format!("{}\n\n", fs::read_to_string(&path).map_err(ReviewError::io(&path))?))
            .map_err(ReviewError::io(&path))?;
        let lines: Vec<Line> = read_lines(&path)?;
        assert_eq!(lines, [Line { id: "a".to_string() }, Line { id: "b".to_string() }]);
        Ok(())
    }

    #[test]
    fn a_missing_file_has_no_lines() -> Outcome {
        let folder = TempDir::new()?;
        let lines: Vec<Line> = read_lines(&folder.path().join("none.jsonl"))?;
        assert_eq!(lines, []);
        Ok(())
    }

    #[test]
    fn a_bad_line_is_named_by_its_number() -> Outcome {
        let folder = TempDir::new()?;
        let path = folder.path().join("bad.jsonl");
        fs::write(&path, "{\"id\":\"a\"}\n\nnot json\n").map_err(ReviewError::io(&path))?;
        let outcome: Outcome<Vec<Line>> = read_lines(&path);
        assert!(outcome.is_err_and(|error| error.to_string().contains("bad.jsonl:3:")));
        Ok(())
    }

    #[test]
    fn a_folder_in_place_of_the_file_is_an_io_error() -> Outcome {
        let folder = TempDir::new()?;
        let outcome: Outcome<Vec<Line>> = read_lines(folder.path());
        assert!(matches!(outcome, Err(ReviewError::Io { .. })));
        Ok(())
    }
}
