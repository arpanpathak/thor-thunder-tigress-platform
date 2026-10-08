//! Every dataset the review page can show, each with its own flags.
//!
//! One server, one port: the page switches between datasets, and every API
//! request names the one it is about with `?dataset=NAME`.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    app::App,
    error::{Outcome, ReviewError},
};

/// Where one dataset's files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetSpec {
    /// The name the page and the API use, such as `train`.
    pub name: String,
    /// The JSONL file of examples.
    pub records: PathBuf,
    /// The reviewer's flags for those examples.
    pub flags: PathBuf,
    /// The examples a build removed as slop, if the dataset has such a file.
    pub removed: PathBuf,
}

impl DatasetSpec {
    /// A dataset named `name` with its three files.
    #[must_use]
    pub fn new(name: &str, records: &Path, flags: &Path, removed: &Path) -> Self {
        Self {
            name: name.to_string(),
            records: records.to_path_buf(),
            flags: flags.to_path_buf(),
            removed: removed.to_path_buf(),
        }
    }
}

/// One open dataset.
pub struct Dataset {
    /// The name the page and the API use.
    pub name: String,
    /// Its index and flags.
    pub app: App,
}

/// Every dataset that could be opened, in the order given.
pub struct Workspace {
    datasets: Vec<Dataset>,
    missing: Vec<String>,
}

/// One dataset as the page's switcher lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatasetInfo {
    /// The name to send back as `?dataset=`.
    pub name: String,
    /// The records file, for the reviewer's reference.
    pub file: String,
    /// How many records it has.
    pub records: usize,
    /// How many of them carry a flag.
    pub flagged: usize,
}

impl Workspace {
    /// Opens every dataset whose records file exists. The others are listed
    /// as missing, so a dataset that has not been built yet does not stop the
    /// server.
    ///
    /// # Errors
    ///
    /// `ReviewError::Io` or `ReviewError::Json` when a file that exists can't
    /// be read, and `ReviewError::NotFound` when no dataset could be opened.
    pub fn open(specs: &[DatasetSpec]) -> Outcome<Workspace> {
        let mut datasets = Vec::new();
        let mut missing = Vec::new();

        for spec in specs {
            if !spec.records.is_file() {
                missing.push(format!("{} ({})", spec.name, spec.records.display()));

                continue;
            }

            let app = App::open(&spec.records, &spec.flags, &spec.removed)?;
            datasets.push(Dataset {
                name: spec.name.clone(),
                app,
            });
        }

        if datasets.is_empty() {
            return Err(ReviewError::NotFound(format!(
                "no dataset to show: {}",
                missing.join(", ")
            )));
        }

        Ok(Workspace { datasets, missing })
    }

    /// The dataset called `name`, or the first one when no name is given.
    ///
    /// # Errors
    ///
    /// `ReviewError::NotFound` for a name no open dataset has.
    pub fn dataset(&self, name: Option<&str>) -> Outcome<&Dataset> {
        let found = match name {
            Some(wanted) => self.datasets.iter().find(|dataset| dataset.name == wanted),
            None => self.datasets.first(),
        };
        found.ok_or_else(|| ReviewError::NotFound(format!("dataset {}", name.unwrap_or_default())))
    }

    /// Every open dataset with its counts.
    ///
    /// # Errors
    ///
    /// `ReviewError::Poisoned` when a thread panicked while holding a dataset's flags.
    pub fn infos(&self) -> Outcome<Vec<DatasetInfo>> {
        self.datasets
            .iter()
            .map(|dataset| {
                Ok(DatasetInfo {
                    name: dataset.name.clone(),
                    file: dataset.app.index.path().display().to_string(),
                    records: dataset.app.index.len(),
                    flagged: dataset.app.flags()?.len(),
                })
            })
            .collect()
    }

    /// The datasets that were asked for but whose records file does not exist.
    #[must_use]
    pub fn missing(&self) -> &[String] {
        &self.missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn spec(folder: &TempDir, name: &str) -> DatasetSpec {
        let path = folder.path();
        DatasetSpec::new(
            name,
            &path.join(format!("{name}.jsonl")),
            &path.join(format!("{name}-flags.jsonl")),
            &path.join("removed.jsonl"),
        )
    }

    #[test]
    fn opens_what_exists_and_lists_what_is_missing() -> Outcome {
        let folder = TempDir::new()?;
        let line = "{\"id\":\"a\",\"source\":\"chat\",\"origin\":\"c\"}\n";
        folder.file("train.jsonl", line)?;
        folder.file("teacher.jsonl", "{\"id\":\"t\",\"source\":\"teacher\",\"origin\":\"trpl/src/a.md\"}\n{\"id\":\"u\",\"source\":\"teacher\",\"origin\":\"x\"}\n")?;
        let workspace = Workspace::open(&[
            spec(&folder, "train"),
            spec(&folder, "teacher"),
            spec(&folder, "conversations"),
        ])?;
        let names: Vec<(String, usize)> = workspace
            .infos()?
            .into_iter()
            .map(|info| (info.name, info.records))
            .collect();
        assert_eq!(
            names,
            [("train".to_string(), 1), ("teacher".to_string(), 2)]
        );
        assert!(workspace.missing()[0].starts_with("conversations ("));
        assert_eq!(workspace.dataset(None)?.name, "train");
        assert_eq!(workspace.dataset(Some("teacher"))?.name, "teacher");
        assert!(matches!(
            workspace.dataset(Some("nope")),
            Err(ReviewError::NotFound(_))
        ));
        Ok(())
    }

    #[test]
    fn nothing_to_open_is_an_error() -> Outcome {
        let folder = TempDir::new()?;
        assert!(matches!(
            Workspace::open(&[spec(&folder, "train")]),
            Err(ReviewError::NotFound(_))
        ));
        Ok(())
    }
}
