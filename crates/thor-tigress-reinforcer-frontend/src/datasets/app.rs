//! What every connection shares: the index, the flags, and the removed examples.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

use serde::Deserialize;

use crate::{
    error::{Outcome, ReviewError},
    review::flags::FlagStore,
    datasets::{
        index::{Ids, Index},
        jsonl,
        workspace::DatasetSpec,
    },
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
    /// Opens the records, flags and removed-examples files named by `spec`.
    /// Missing flags and removed-examples files read as empty.
    ///
    /// # Errors
    ///
    /// `ReviewError::Io` or `ReviewError::Json` for any unreadable file.
    pub fn open(spec: &DatasetSpec) -> Outcome<App> {
        let removed: Vec<Removed> = jsonl::read_lines(&spec.removed)?;

        Ok(App {
            index: Index::open_as(&spec.records, spec.kind)?,
            flags: Mutex::new(FlagStore::open(&spec.flags)?),
            slop: spec.removed.clone(),
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
        let line = "{\"id\":\"a\",\"source\":\"chat\",\"origin\":\"c\"}\n";
        let training = folder.file("train.jsonl", line)?;
        let flags = folder.path().join("flags.jsonl");
        let slop = folder.path().join("slop.jsonl");
        let app = App::open(&DatasetSpec::new("train", &training, &flags, &slop))?;

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
        let flags = folder.path().join("flags.jsonl");
        let app = App::open(&DatasetSpec::new("train", &training, &flags, &slop))?;

        assert_eq!(app.slop_ids, ["gone".to_string()].into_iter().collect());

        Ok(())
    }
}
