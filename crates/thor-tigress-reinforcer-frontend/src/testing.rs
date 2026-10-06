//! Helpers for tests: a temporary folder that removes itself.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::error::{Outcome, ReviewError};

/// Tells folders made by tests running in parallel apart.
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A fresh folder under the system's temporary folder, removed when dropped.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a new, empty folder.
    pub fn new() -> Outcome<TempDir> {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("reinforcer-test-{}-{number}", std::process::id()));
        fs::create_dir_all(&path).map_err(ReviewError::io(&path))?;
        Ok(TempDir { path })
    }

    /// The folder.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `text` to `name` inside the folder and returns its path.
    pub fn file(&self, name: &str, text: &str) -> Outcome<PathBuf> {
        let path = self.path.join(name);
        fs::write(&path, text).map_err(ReviewError::io(&path))?;
        Ok(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            eprintln!("could not remove {}: {error}", self.path.display());
        }
    }
}
