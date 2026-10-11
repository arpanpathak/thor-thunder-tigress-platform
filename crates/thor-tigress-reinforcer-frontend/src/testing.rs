//! Helpers for tests: a temporary folder that removes itself.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::error::{Outcome, ReviewError};

/// A fresh folder under the system's temporary folder, removed when dropped.
pub struct TempDir(tempfile::TempDir);

impl TempDir {
    /// Creates a new, empty folder.
    pub fn new() -> Outcome<TempDir> {
        let folder = tempfile::tempdir().map_err(ReviewError::io(Path::new("temporary folder")))?;

        Ok(TempDir(folder))
    }

    /// The folder.
    pub fn path(&self) -> &Path {
        self.0.path()
    }

    /// Writes `text` to `name` inside the folder and returns its path.
    pub fn file(&self, name: &str, text: &str) -> Outcome<PathBuf> {
        let path = self.path().join(name);
        fs::write(&path, text).map_err(ReviewError::io(&path))?;

        Ok(path)
    }
}
