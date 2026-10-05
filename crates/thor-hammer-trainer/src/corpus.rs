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
//! [`crate::book::examples`], after YAML front matter and Hugo shortcodes are
//! removed.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use regex::Regex;

use crate::{book, error::DataError, example::{Example, Source}};

/// Where the fetcher puts the sources.
pub const CORPUS_DIRECTORY: &str = "corpus";

/// The manifest the fetcher writes, committed so the record survives.
pub const MANIFEST_FILE: &str = "train/corpus.manifest.tsv";

/// Sources that are not mdBook projects, and the sub-tree of each that holds
/// the English prose; an empty sub-tree means the whole checkout. Anything
/// outside it is a translation, an old version, or site furniture. A source
/// can be listed more than once.
const PLAIN_SOURCES: [(&str, &str); 16] = [
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
];

/// Sources whose `README.md` files are the content rather than a table of
/// contents: the System Design Primer is one long README, and its worked
/// solutions are READMEs too.
const README_IS_CONTENT: [&str; 1] = ["system-design-primer"];

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
const RESTRICTING_TERMS: [&str; 5] = ["NonCommercial", "NoDerivatives", "NoDerivs", "ShareAlike", "Non-Commercial"];

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
        let restricted = creative_commons && RESTRICTING_TERMS.iter().any(|term| text.contains(term));
        match text {
            _ if text.contains("Mozilla Public License") => Licence::MozillaPublic,
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
            Licence::Mit | Licence::Apache | Licence::Bsd | Licence::CreativeCommons
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

/// Reads every accepted source under `root` and turns it into examples.
pub fn examples(root: &Path, manifest_path: &Path) -> Result<Corpus, DataError> {
    let mut examples = Vec::new();
    let mut reports = Vec::new();
    for row in read_manifest(manifest_path)? {
        let source_dir = root.join(&row.name);
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
            produced.extend(book::examples(&text, &origin).into_iter().map(as_corpus_example));
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
    let tokens = |example: &Example| example.char_count() / 4;
    let mut by_id: Vec<(String, usize)> = examples
        .iter()
        .enumerate()
        .map(|(position, example)| (example.id(), position))
        .collect();
    by_id.sort();
    let mut total = 0usize;
    let mut chosen = vec![false; examples.len()];
    for (_, position) in by_id {
        let size = examples.get(position).map_or(0, tokens);
        if total + size > limit {
            continue;
        }
        total += size;
        if let Some(slot) = chosen.get_mut(position) {
            *slot = true;
        }
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
fn collect_markdown(root: &Path, keep_readme: bool, files: &mut Vec<PathBuf>) -> Result<(), DataError> {
    if !root.is_dir() {
        return Ok(());
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(DataError::io(&directory))? {
            let path = entry.map_err(DataError::io(&directory))?.path();
            let is_hidden = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'));
            if is_hidden {
                continue;
            }
            match path.is_dir() {
                true => pending.push(path),
                false => {
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default();
                    let is_markdown = path
                        .extension()
                        .is_some_and(|extension| extension == "md" || extension == "markdown");
                    let is_navigation = NAVIGATION_FILES.contains(&name) && !(keep_readme && name == "README.md");
                    if is_markdown && !is_navigation {
                        files.push(path);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Removes YAML front matter, Hugo shortcodes, HTML comments and link
/// references, which are site furniture rather than prose and would otherwise
/// be learned as part of an answer, and joins hard-wrapped lines back into
/// paragraphs.
pub fn clean(markdown: &str) -> String {
    let without_front_matter = strip_front_matter(markdown);
    let has_heading = without_front_matter
        .lines()
        .any(|line| line.starts_with("# ") || line.starts_with("## "));
    let titled = match (front_matter_title(markdown), has_heading) {
        (Some(title), false) => format!("# {title}\n\n{without_front_matter}"),
        _ => without_front_matter.to_string(),
    };
    let without_front_matter = titled.as_str();
    let without_comments = HTML_COMMENT
        .as_ref()
        .map_or(without_front_matter.to_string(), |comment| {
            comment.replace_all(without_front_matter, "").into_owned()
        });
    let without_comments = HEADING_ANCHOR
        .as_ref()
        .map_or(without_comments.clone(), |anchor| {
            anchor.replace_all(&without_comments, "$1").into_owned()
        });
    let stripped = without_comments
        .lines()
        .map(strip_shortcodes)
        .map(|line| strip_html(&line))
        .collect::<Vec<String>>()
        .join("\n");
    collapse_blank_lines(&tidy_prose(&stripped))
}

static HEADING_ANCHOR: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?m)^(#{1,6} .*?)\s*\{#[\w.-]+\}[ \t]*$").ok());

static HTML_COMMENT: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").ok());

static LINK_DEFINITION: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\s{0,3}\[[^\]]+\]:\s*\S+").ok());

static REFERENCE_LINK: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\[([^\]\n]+)\]\[[^\]\n]*\]").ok());

static BLOCK_START: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\s*(?:#|[-*+]\s|\d+[.)]\s|>|\||```|~~~|<)").ok());

/// Outside code blocks: drops link definitions such as `[raw]: #raw`, turns
/// reference links such as `[Raw Identifiers][raw]` into their text, and joins
/// a line that continues the paragraph or list item above it. Lines inside
/// code blocks are kept exactly, so `grid[i][j]` is never read as a link.
fn tidy_prose(text: &str) -> String {
    let (Some(definition), Some(reference), Some(block_start)) =
        (LINK_DEFINITION.as_ref(), REFERENCE_LINK.as_ref(), BLOCK_START.as_ref())
    else {
        return text.to_string();
    };
    let mut lines: Vec<(bool, String)> = Vec::new();
    let mut inside_code = false;
    let mut joinable = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let is_fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if inside_code || is_fence {
            inside_code ^= is_fence;
            lines.push((true, line.to_string()));
            joinable = false;
            continue;
        }
        if definition.is_match(line) {
            continue;
        }
        let continues = joinable && !trimmed.is_empty() && !block_start.is_match(line);
        match (continues, lines.last_mut()) {
            (true, Some((_, previous))) => {
                previous.push(' ');
                previous.push_str(trimmed);
            }
            _ => lines.push((false, line.to_string())),
        }
        joinable = !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with('|');
    }
    lines
        .into_iter()
        .map(|(is_code, line)| match is_code {
            true => line,
            false => reference.replace_all(&line, "$1").into_owned(),
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// The HTML element names the sources actually use.
///
/// A tag is only removed when its name is in this list. That matters: `Vec<i32>`
/// and `<T>` are angle brackets too, and a rule that stripped every `<...>` would
/// quietly delete Rust generics out of the training data.
const HTML_ELEMENTS: [&str; 38] = [
    "a", "b", "blockquote", "br", "caption", "code", "dd", "div", "dl", "dt", "em", "figcaption",
    "figure", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "i", "iframe", "img", "li", "ol", "p",
    "pre", "source", "span", "strong", "sub", "sup", "table", "tbody", "td", "th", "thead", "tr",
];

/// Removes HTML tags but keeps the text between them, then decodes the entities
/// the sources use.
///
/// Book markup learned as output teaches the model to answer in HTML. One
/// reviewer flagged an example by hand for exactly this, and the same markup
/// appears in thousands, so it is removed at the source rather than reviewed one
/// example at a time.
fn strip_html(line: &str) -> String {
    let mut kept = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find('<') {
        kept.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(offset) => {
                let tag = &rest[start..start + offset + 1];
                if is_html_tag(tag) {
                    if tag.starts_with("<br") || tag.starts_with("</p") || tag == "<p>" {
                        kept.push(' ');
                    }
                } else {
                    kept.push_str(tag);
                }
                rest = &rest[start + offset + 1..];
            }
            None => {
                kept.push_str(&rest[start..]);
                rest = "";
                break;
            }
        }
    }
    kept.push_str(rest);
    decode_entities(&kept)
}

/// True when `tag` names a known HTML element, ignoring any attributes.
fn is_html_tag(tag: &str) -> bool {
    let body = tag.strip_prefix("</").or_else(|| tag.strip_prefix('<')).unwrap_or(tag);
    let name: String = body
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    !name.is_empty() && HTML_ELEMENTS.contains(&name.as_str())
}

/// Turns the handful of HTML entities the sources use back into characters.
fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// Squeezes runs of blank lines, which removing a tag can leave behind.
fn collapse_blank_lines(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut blanks = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        kept.push(line);
    }
    kept.join("\n")
}

/// The `title:` of a leading `---` front matter block, unquoted.
fn front_matter_title(markdown: &str) -> Option<String> {
    let mut lines = markdown.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return None;
    }
    lines
        .take_while(|line| line.trim_end() != "---")
        .find_map(|line| line.strip_prefix("title:"))
        .map(|title| title.trim().trim_matches(['"', '\'']).to_string())
        .filter(|title| !title.is_empty())
}

/// Removes a leading `---` block.
fn strip_front_matter(markdown: &str) -> &str {
    let mut lines = markdown.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return markdown;
    }
    let mut consumed = 4;
    for line in lines {
        consumed += line.len() + 1;
        if line.trim_end() == "---" {
            return markdown.get(consumed..).unwrap_or("");
        }
    }
    markdown
}

/// Removes `{{< ... >}}` and `{{% ... %}}` from one line.
fn strip_shortcodes(line: &str) -> String {
    let mut kept = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find("{{") {
        kept.push_str(&rest[..start]);
        let closing = rest[start..]
            .find("}}")
            .map(|offset| start + offset + 2)
            .unwrap_or(rest.len());
        kept.push_str(&rest[start..closing].chars().map(|_| ' ').collect::<String>());
        rest = &rest[closing..];
    }
    kept.push_str(rest);
    kept.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_heading_anchors() {
        assert_eq!(clean("# Processes and the Kernel {#processes}\n\nText.\n"), "# Processes and the Kernel\n\nText.");
    }

    #[test]
    fn a_page_without_a_heading_takes_its_front_matter_title() {
        let cleaned = clean("---\ntitle: \"Pod Lifecycle\"\nweight: 30\n---\nPods follow a lifecycle.\n");
        assert!(cleaned.starts_with("# Pod Lifecycle\n\nPods follow"), "{cleaned}");
    }

    #[test]
    fn tidies_book_prose_but_leaves_code_alone() {
        let markdown = "Use raw identifiers, as in the [\u{201c}Raw\nIdentifiers\u{201d}][raw]<!-- ignore --> section.\n\n[raw]: #raw\n\n- `as`: casting, or rename\n  items in `use`.\n\n```rust\nlet x = grid[i][j]; // [a]: b\n```\n";
        let cleaned = clean(markdown);
        assert!(cleaned.contains("as in the \u{201c}Raw Identifiers\u{201d} section."), "{cleaned}");
        assert!(!cleaned.contains("[raw]: #raw"));
        assert!(!cleaned.contains("ignore"));
        assert!(cleaned.contains("- `as`: casting, or rename items in `use`."));
        assert!(cleaned.contains("let x = grid[i][j]; // [a]: b"));
    }

    #[test]
    fn caps_a_source_by_tokens_and_keeps_the_original_order() {
        let section = |text: &str| Example {
            instruction: String::new(),
            response: text.repeat(40),
            source: Source::Corpus,
            origin: String::new(),
        };
        let all: Vec<Example> = ["a", "b", "c", "d"].iter().map(|text| section(text)).collect();
        let kept = cap_tokens(all.clone(), 20);
        assert_eq!(kept.len(), 2);
        let positions: Vec<usize> = kept
            .iter()
            .filter_map(|example| all.iter().position(|candidate| candidate == example))
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(cap_tokens(all.clone(), 1_000).len(), 4);
    }

    #[test]
    fn classifies_the_licences_actually_fetched() {
        assert_eq!(
            Licence::classify("Apache License Version 2.0, January 2004"),
            Licence::Apache
        );
        assert_eq!(
            Licence::classify("Copyright (c) 2015 Aria Desires Permission is hereby granted"),
            Licence::Mit
        );
        assert_eq!(
            Licence::classify("Copyright 2009 The Go Authors. Redistribution and use in source and binary forms"),
            Licence::Bsd
        );
        assert_eq!(
            Licence::classify("Attribution 4.0 International"),
            Licence::CreativeCommons
        );
        assert_eq!(
            Licence::classify("Mozilla Public License Version 2.0"),
            Licence::MozillaPublic
        );
        assert_eq!(
            Licence::classify("Attribution-NonCommercial-NoDerivatives 4.0 International"),
            Licence::CreativeCommonsRestricted
        );
        assert_eq!(
            Licence::classify("Creative Commons Legal Code Attribution-ShareAlike 3.0 Unported"),
            Licence::CreativeCommonsRestricted
        );
        assert_eq!(
            Licence::classify("Creative Commons Legal Code CC0 1.0 Universal"),
            Licence::CreativeCommons
        );
        assert_eq!(
            Licence::classify("CC0 1.0 Universal Statement of Purpose The laws of most jurisdictions"),
            Licence::CreativeCommons
        );
        assert_eq!(
            Licence::classify("MIT License Permission is hereby granted || Attribution-NonCommercial 4.0 International"),
            Licence::CreativeCommonsRestricted
        );
    }

    #[test]
    fn mozilla_public_is_refused() {
        assert!(Licence::Mit.permits_reuse());
        assert!(Licence::Apache.permits_reuse());
        assert!(!Licence::MozillaPublic.permits_reuse());
        assert!(!Licence::Missing.permits_reuse());
        assert!(!Licence::Unrecognised.permits_reuse());
    }

    #[test]
    fn reads_the_source_directory_from_book_toml() {
        assert_eq!(book_source_in("[book]\nsrc = \"src\"\n"), "src");
        assert_eq!(book_source_in("[book]\ntitle = \"x\"\n"), DEFAULT_BOOK_SOURCE);
        assert_eq!(book_source_in("[book]\nsrc = 'book-src'\n"), "book-src");
    }

    #[test]
    fn removes_front_matter_and_shortcodes() {
        let markdown = "---\ntitle: x\n---\n\n# Head\n\n{{< note >}}\n\nText.\n";
        let cleaned = clean(markdown);
        assert!(cleaned.starts_with("\n# Head"));
        assert!(!cleaned.contains("title: x"));
        assert!(!cleaned.contains("{{"));
        assert!(cleaned.contains("Text."));
    }

    #[test]
    fn keeps_markdown_without_front_matter() {
        assert_eq!(clean("# Head\n\nText.\n"), "# Head\n\nText.");
    }

    #[test]
    fn removes_source_markup_but_keeps_the_text() {
        assert_eq!(
            strip_html("<span class=\"caption\">Table B-1: Operators</span>"),
            "Table B-1: Operators"
        );
        assert_eq!(strip_html("a<br>b"), "a b");
    }

    #[test]
    fn keeps_rust_generics() {
        assert_eq!(strip_html("a Vec<i32> and a HashMap<K, V>"), "a Vec<i32> and a HashMap<K, V>");
        assert_eq!(strip_html("`Result<T, E>`"), "`Result<T, E>`");
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(strip_html("a &amp; b"), "a & b");
        assert_eq!(strip_html("x &lt; y"), "x < y");
    }
}
