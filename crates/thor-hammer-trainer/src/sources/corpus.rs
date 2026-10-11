//! Examples from the open corpus fetched by `train/fetch_corpus.sh`.
//!
//! ## Design
//!
//! The fetcher writes `train/corpus.manifest.tsv`: one row per source, with the
//! commit, the licence file and the licence text read from that file. This
//! module reads the manifest and refuses any source whose licence is not on the
//! allow list, so a source cannot enter the training set without its licence
//! having been read.
//!
//! Where the prose lives is taken from the source itself rather than from a
//! table here:
//!
//! * An mdBook project declares its source directory in `book.toml`. That
//!   matters more than it looks: `rust-lang/book` carries `first-edition`,
//!   `second-edition` and `2018-edition` beside the current `src`, and reading
//!   the tree blindly would train on three superseded copies of the same book.
//! * Documentation sites that are not books are listed in [`PLAIN_SOURCES`]
//!   with the English sub-tree to read.
//!
//! Each markdown file becomes one example per section, through
//! [`crate::sources::book::examples`], after YAML front matter and Hugo shortcodes are
//! removed.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    sources::book,
    error::DataError,
    example::{Example, Source},
};

#[path = "corpus_markdown.rs"]
mod markdown;

pub use self::markdown::clean;

/// Where the fetcher puts the sources.
pub const CORPUS_DIRECTORY: &str = "corpus";

/// The manifest the fetcher writes, committed so the record survives.
pub const MANIFEST_FILE: &str = "train/corpus.manifest.tsv";

/// Sources that are not mdBook projects, and the sub-tree of each that holds
/// the English prose; an empty sub-tree means the whole checkout. Anything
/// outside it is a translation, an old version, or site furniture. A source
/// can be listed more than once.
const PLAIN_SOURCES: [(&str, &str); 18] = [
    ("kubernetes-website", "content/en"),
    ("go-website", "_content"),
    ("ms-style-guide", "styleguide"),
    ("rust-rfcs", "text"),
    ("tokio-website", "content/tokio"),
    ("tigerbeetle", "docs"),
    ("etcd-website", "content/en/docs/v3.6"),
    ("prometheus-docs", "docs"),
    ("grpc-website", "content/en/docs"),
    ("aosa-500lines", ""),
    ("system-design-primer", ""),
    ("eng-practices", "review"),
    ("ms-api-guidelines", "azure"),
    ("ms-api-guidelines", "graph"),
    ("twelve-factor", "content/en"),
    ("google-styleguide", ""),
    ("uber-go-guide", "src"),
    ("airbnb-javascript", ""),
];

/// Sources whose `README.md` files are the content rather than a table of
/// contents: the System Design Primer is one long README, and its worked
/// solutions are READMEs too; the Airbnb JavaScript guide is a README per topic.
const README_IS_CONTENT: [&str; 2] = ["system-design-primer", "airbnb-javascript"];

/// Parts of a source that are not prose worth learning from: the Kubernetes
/// blog is release announcements, and its reference pages are generated from
/// code (API tables, command flags, config schemas).
const SKIPPED_TREES: [&str; 12] = [
    "aosa-500lines/incomplete",
    "aosa-500lines/BUILD.md",
    "aosa-500lines/_build",
    "system-design-primer/README-",
    "system-design-primer/CONTRIBUTING.md",
    "system-design-primer/TRANSLATIONS.md",
    "kubernetes-website/content/en/blog",
    "kubernetes-website/content/en/docs/reference/kubernetes-api",
    "kubernetes-website/content/en/docs/reference/generated",
    "kubernetes-website/content/en/docs/reference/kubectl/generated",
    "kubernetes-website/content/en/docs/reference/setup-tools/kubeadm/generated",
    "kubernetes-website/content/en/docs/reference/config-api",
];

/// How deep to look for a `book.toml` below a source root.
const MAX_BOOK_DEPTH: usize = 4;

/// The source directory an mdBook uses when `book.toml` does not say.
const DEFAULT_BOOK_SOURCE: &str = "src";

/// Files that list other files rather than saying anything themselves. A table
/// of contents is the one thing in a book that is never an answer.
const NAVIGATION_FILES: [&str; 2] = ["SUMMARY.md", "README.md"];

/// Creative Commons terms that rule a licence out: no commercial use, no
/// derived works, or derived works under the same licence.
const RESTRICTING_TERMS: [&str; 5] = [
    "NonCommercial",
    "NoDerivatives",
    "NoDerivs",
    "ShareAlike",
    "Non-Commercial",
];

/// The licence of one fetched source, decided from the text of its licence file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Licence {
    /// MIT.
    Mit,
    /// Apache-2.0.
    Apache,
    /// BSD, any variant.
    Bsd,
    /// Creative Commons Attribution (CC BY) or CC0, with no further terms.
    CreativeCommons,
    /// A Creative Commons licence with NonCommercial, NoDerivatives or
    /// ShareAlike terms. Training on such text is not clearly allowed.
    CreativeCommonsRestricted,
    /// Mozilla Public License 2.0, which is file-level copyleft.
    MozillaPublic,
    /// Python Software Foundation License 2, a permissive licence.
    PythonSoftwareFoundation,
    /// A licence file that was present but not recognised.
    Unrecognised,
    /// No licence file at all.
    Missing,
}

impl Licence {
    /// Decides the licence from the text of a source's licence files, joined
    /// by ` || ` when there are several. A restrictive file wins over a
    /// permissive one: code under MIT does not make NonCommercial text usable.
    pub fn classify(text: &str) -> Self {
        let creative_commons = text.contains("Creative Commons")
            || text.contains("Attribution")
            || text.contains("CC0 1.0 Universal");
        let restricted =
            creative_commons && RESTRICTING_TERMS.iter().any(|term| text.contains(term));

        match text {
            _ if text.contains("Mozilla Public License") => Licence::MozillaPublic,
            _ if text.contains("PYTHON SOFTWARE FOUNDATION LICENSE VERSION 2") => {
                Licence::PythonSoftwareFoundation
            }
            _ if restricted => Licence::CreativeCommonsRestricted,
            _ if text.contains("Apache License") => Licence::Apache,
            _ if text.contains("Permission is hereby granted") || text.contains("MIT License") => {
                Licence::Mit
            }
            _ if text.contains("Redistribution and use in source and binary forms") => Licence::Bsd,
            _ if creative_commons => Licence::CreativeCommons,
            _ if text.contains("NO-LICENCE-FILE") || text.is_empty() => Licence::Missing,
            _ => Licence::Unrecognised,
        }
    }

    /// True when the source may be trained on.
    ///
    /// Mozilla Public is excluded by default: it is file-level copyleft rather
    /// than a permissive licence, and the project keeps the two apart on
    /// purpose rather than relying on a reading of what training does.
    pub fn permits_reuse(self) -> bool {
        matches!(
            self,
            Licence::Mit
                | Licence::Apache
                | Licence::Bsd
                | Licence::CreativeCommons
                | Licence::PythonSoftwareFoundation
        )
    }

    /// The name used in the report.
    pub fn name(self) -> &'static str {
        match self {
            Licence::Mit => "MIT",
            Licence::Apache => "Apache-2.0",
            Licence::Bsd => "BSD",
            Licence::CreativeCommons => "CC-BY",
            Licence::CreativeCommonsRestricted => "CC with NC/ND/SA terms",
            Licence::MozillaPublic => "MPL-2.0",
            Licence::PythonSoftwareFoundation => "PSF-2.0",
            Licence::Unrecognised => "unrecognised",
            Licence::Missing => "missing",
        }
    }
}

/// One row of the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestRow {
    /// The directory name under `corpus/`.
    pub name: String,
    /// The kind the fetcher recorded.
    pub kind: String,
    /// The commit that was checked out.
    pub commit: String,
    /// The licence read from the licence file.
    pub licence: Licence,
}

/// What one source contributed, or why it did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceOutcome {
    /// Examples were produced.
    Used {
        /// The number of examples kept.
        examples: usize,
        /// The number of examples before [`MAX_SOURCE_TOKENS`] was applied.
        before_cap: usize,
        /// The licence they were accepted under.
        licence: Licence,
    },
    /// The licence does not allow reuse.
    LicenceRefused(Licence),
    /// No markdown was found under the expected directory.
    NoMarkdown,
    /// A code repository (kind `code`): the teacher reads its source files,
    /// so its markdown is not added as prose.
    CodeOnly,
}

/// One source and what came of reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReport {
    /// The source name.
    pub name: String,
    /// What happened.
    pub outcome: SourceOutcome,
}

/// Every example the corpus contributes, and a per-source account of it.
pub struct Corpus {
    /// The examples, ready to join the training set.
    pub examples: Vec<Example>,
    /// One report per manifest row, in manifest order.
    pub reports: Vec<SourceReport>,
}

/// Reads the manifest.
///
/// The columns are `source`, `kind`, `commit`, `licence_file`, `licence`.
pub fn read_manifest(path: &Path) -> Result<Vec<ManifestRow>, DataError> {
    let text = fs::read_to_string(path).map_err(DataError::io(path))?;
    let mut rows = Vec::new();

    for (index, line) in text.lines().enumerate() {
        if index == 0 || line.trim().is_empty() {
            continue;
        }

        let columns: Vec<&str> = line.split('\t').collect();

        if columns.len() < 5 {
            continue;
        }

        rows.push(ManifestRow {
            name: columns[0].to_string(),
            kind: columns[1].to_string(),
            commit: columns[2].to_string(),
            licence: Licence::classify(columns[4]),
        });
    }

    Ok(rows)
}

/// The manifest kind of a repository read for its source code.
pub const CODE_KIND: &str = "code";

/// Reads every accepted source under `root` and turns it into examples.
pub fn examples(root: &Path, manifest_path: &Path) -> Result<Corpus, DataError> {
    let mut examples = Vec::new();
    let mut reports = Vec::new();

    for row in read_manifest(manifest_path)? {
        let source_dir = root.join(&row.name);

        if row.licence.permits_reuse() && row.kind == CODE_KIND {
            reports.push(SourceReport {
                name: row.name,
                outcome: SourceOutcome::CodeOnly,
            });

            continue;
        }

        if !row.licence.permits_reuse() {
            reports.push(SourceReport {
                name: row.name,
                outcome: SourceOutcome::LicenceRefused(row.licence),
            });

            continue;
        }

        if !source_dir.is_dir() {
            reports.push(SourceReport {
                name: row.name,
                outcome: SourceOutcome::NoMarkdown,
            });

            continue;
        }

        let mut produced = Vec::new();

        for file in markdown_roots(&row.name, &source_dir)? {
            let origin = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .display()
                .to_string();

            if SKIPPED_TREES.iter().any(|tree| origin.starts_with(tree)) {
                continue;
            }

            let text = clean(&fs::read_to_string(&file).map_err(DataError::io(&file))?);

            produced.extend(
                book::examples(&text, &origin)
                    .into_iter()
                    .map(as_corpus_example),
            );
        }

        let before_cap = produced.len();
        let kept = cap_tokens(produced, MAX_SOURCE_TOKENS);

        reports.push(SourceReport {
            name: row.name,
            outcome: match kept.len() {
                0 => SourceOutcome::NoMarkdown,
                count => SourceOutcome::Used {
                    examples: count,
                    before_cap,
                    licence: row.licence,
                },
            },
        });

        examples.extend(kept);
    }

    Ok(Corpus { examples, reports })
}

/// The most tokens one source may put in the training set, estimated as
/// characters / 4 like the report does. Without it the Rust RFCs, Kubernetes
/// and Go docs were half of all training tokens, and the model would learn to
/// write like them before it learned to write like the Rust books.
pub const MAX_SOURCE_TOKENS: usize = 400_000;

/// At most `limit` tokens of `examples`. The sections kept are chosen by id,
/// a hash of their text, so the choice spreads over the whole source rather
/// than taking its first chapters, and it is the same on every build. The kept
/// sections stay in their original order.
fn cap_tokens(examples: Vec<Example>, limit: usize) -> Vec<Example> {
    let mut by_id: Vec<(String, usize, usize)> = examples
        .iter()
        .enumerate()
        .map(|(position, example)| (example.id(), position, example.char_count() / 4))
        .collect();
    by_id.sort();

    let mut total = 0usize;
    let mut chosen = vec![false; examples.len()];

    for (_, position, size) in by_id {
        if total + size > limit {
            continue;
        }

        total += size;
        chosen[position] = true;
    }

    examples
        .into_iter()
        .zip(chosen)
        .filter_map(|(example, keep)| keep.then_some(example))
        .collect()
}

/// Marks an example as coming from the fetched open corpus rather than from the
/// book repository, so the report can weigh the two apart.
fn as_corpus_example(mut example: Example) -> Example {
    example.source = Source::Corpus;

    example
}

/// Every markdown file of one source, in a stable order.
///
/// An mdBook project is read from the directory its `book.toml` names, so only
/// the current edition is read. A documentation site is read from the
/// sub-tree listed in [`PLAIN_SOURCES`].
fn markdown_roots(source: &str, source_dir: &Path) -> Result<Vec<PathBuf>, DataError> {
    let mut roots = Vec::new();

    for book_root in book_source_directories(source_dir)? {
        collect_markdown(&book_root, false, &mut roots)?;
    }

    let keep_readme = README_IS_CONTENT.contains(&source);

    for (name, sub_tree) in PLAIN_SOURCES {
        if name == source {
            collect_markdown(&source_dir.join(sub_tree), keep_readme, &mut roots)?;
        }
    }

    roots.sort();
    roots.dedup();

    Ok(roots)
}

/// The source directory of every mdBook found under `source_dir`.
fn book_source_directories(source_dir: &Path) -> Result<Vec<PathBuf>, DataError> {
    let mut directories = Vec::new();
    let mut pending = vec![(source_dir.to_path_buf(), 0usize)];

    while let Some((directory, depth)) = pending.pop() {
        let book_toml = directory.join("book.toml");

        if book_toml.is_file() {
            let text = fs::read_to_string(&book_toml).map_err(DataError::io(&book_toml))?;
            directories.push(directory.join(book_source_in(&text)));
            continue;
        }

        if depth >= MAX_BOOK_DEPTH {
            continue;
        }

        for entry in fs::read_dir(&directory).map_err(DataError::io(&directory))? {
            let path = entry.map_err(DataError::io(&directory))?.path();
            let is_hidden = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'));

            if path.is_dir() && !is_hidden {
                pending.push((path, depth + 1));
            }
        }
    }

    Ok(directories)
}

/// The `src` value of a `book.toml`, defaulting to [`DEFAULT_BOOK_SOURCE`].
fn book_source_in(book_toml: &str) -> String {
    book_toml
        .lines()
        .filter_map(|line| line.trim().strip_prefix("src"))
        .filter_map(|rest| rest.trim().strip_prefix('='))
        .map(|value| value.trim().trim_matches('"').trim_matches('\''))
        .find(|value| !value.is_empty())
        .unwrap_or(DEFAULT_BOOK_SOURCE)
        .to_string()
}

/// Collects every `.md` file under `root`, skipping hidden directories.
fn collect_markdown(
    root: &Path,
    keep_readme: bool,
    files: &mut Vec<PathBuf>,
) -> Result<(), DataError> {
    if !root.is_dir() {
        return Ok(());
    }

    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(DataError::io(&directory))? {
            let path = entry.map_err(DataError::io(&directory))?.path();

            if file_name(&path).starts_with('.') {
                continue;
            }

            match path.is_dir() {
                true => pending.push(path),
                false if is_prose(&path, keep_readme) => files.push(path),
                false => {}
            }
        }
    }

    Ok(())
}

/// The last part of `path` as text, or nothing when it is not UTF-8.
fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

/// True for a markdown file that holds prose instead of a table of contents.
fn is_prose(path: &Path, keep_readme: bool) -> bool {
    let name = file_name(path);
    let is_markdown = path
        .extension()
        .is_some_and(|extension| extension == "md" || extension == "markdown");
    let is_navigation = NAVIGATION_FILES.contains(&name) && !(keep_readme && name == "README.md");

    is_markdown && !is_navigation
}

#[cfg(test)]
#[path = "corpus_tests.rs"]
mod tests;
