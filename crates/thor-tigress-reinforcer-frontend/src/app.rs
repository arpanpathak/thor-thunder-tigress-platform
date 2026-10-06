//! What every connection shares: the index, the flags, and the removed examples.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use serde::Deserialize;

use crate::{
    error::{Outcome, ReviewError},
    flags::FlagStore,
    index::{Ids, Index},
    jsonl,
};

/// The index and the flags, shared by every connection.
pub struct App {
    /// The training file.
    pub index: Index,
    flags: Mutex<FlagStore>,
    /// The examples a previous build removed as slop.
    pub slop: PathBuf,
    /// Their ids, so a flag that points at one is not mistaken for an orphan.
    pub slop_ids: Ids,
}

/// A line of the removed-examples file, as far as the tool reads it.
#[derive(Deserialize)]
struct Removed {
    id: Option<String>,
}

impl App {
    /// Opens the training file, the flags and the removed examples.
    ///
    /// # Errors
    ///
    /// `ReviewError::Io` or `ReviewError::Json` for any unreadable file.
    pub fn open(training: &Path, flags: &Path, slop: &Path) -> Outcome<App> {
        let removed: Vec<Removed> = jsonl::read_lines(slop)?;
        Ok(App {
            index: Index::open(training)?,
            flags: Mutex::new(FlagStore::open(flags)?),
            slop: slop.to_path_buf(),
            slop_ids: removed.into_iter().filter_map(|line| line.id).collect(),
        })
    }

    /// The flags, locked for this request.
    ///
    /// # Errors
    ///
    /// `ReviewError::Poisoned` when a thread panicked while holding them.
    pub fn flags(&self) -> Outcome<MutexGuard<'_, FlagStore>> {
        self.flags.lock().map_err(|_| ReviewError::Poisoned)
    }

    /// The ids that carry a flag.
    ///
    /// # Errors
    ///
    /// As for [`App::flags`].
    pub fn flagged_ids(&self) -> Outcome<HashSet<String>> {
        Ok(self.flags()?.ids())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn opens_with_missing_flag_and_slop_files() -> Outcome {
        let folder = TempDir::new()?;
        let training = folder.file("train.jsonl", "{\"id\":\"a\",\"source\":\"chat\",\"origin\":\"c\"}\n")?;
        let app = App::open(&training, &folder.path().join("flags.jsonl"), &folder.path().join("slop.jsonl"))?;
        assert_eq!(app.index.len(), 1);
        assert_eq!(app.flagged_ids()?.len(), 0);
        assert!(app.slop_ids.is_empty());
        Ok(())
    }

    #[test]
    fn remembers_the_ids_of_removed_examples() -> Outcome {
        let folder = TempDir::new()?;
        let training = folder.file("train.jsonl", "")?;
        let slop = folder.file("slop.jsonl", "{\"id\":\"gone\"}\n{\"other\":1}\n")?;
        let app = App::open(&training, &folder.path().join("flags.jsonl"), &slop)?;
        assert_eq!(app.slop_ids, ["gone".to_string()].into_iter().collect());
        Ok(())
    }
}
