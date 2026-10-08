//! Picks real sections for the teacher to write conversations from.
//!
//! A section is either a passage of prose, from the book and corpus passages
//! already in `data/train.jsonl`, or a source file from one of the corpus's
//! code repositories (manifest kind `code`). Each carries its id, file and
//! licence. The teacher reads a picked section and writes conversations
//! grounded in it, naming the section's id in the entry (see [`crate::teacher`]).

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write as _,
    fs,
    path::Path,
};

use serde::Deserialize;

use crate::{corpus, error::DataError, example::stable_id};

/// Sections shorter than this, in characters, rarely hold enough to ask about.
pub const MIN_SECTION_CHARS: usize = 800;

/// Prose sections longer than this are cut in the queue; the teacher reads the
/// file for the rest. Source files are queued whole.
pub const MAX_QUEUE_CHARS: usize = 6000;

/// The licence named for the author's own books, which are not in the corpus manifest.
pub const AUTHORS_OWN: &str = "author's own";

/// Sources the teacher does not write from.
pub const EXCLUDED_SOURCES: [&str; 1] = ["gpu-accelerated-kubernetes"];

/// Source files shorter than this, in characters, are mostly boilerplate.
pub const MIN_CODE_CHARS: usize = 1_500;

/// Source files longer than this are too long to teach from as one piece.
pub const MAX_CODE_CHARS: usize = 24_000;

/// File extensions read from code repositories, with the fence tag for each.
const CODE_LANGUAGES: [(&str, &str); 11] = [
    ("rs", "rust"),
    ("go", "go"),
    ("py", "python"),
    ("java", "java"),
    ("js", "javascript"),
    ("ts", "typescript"),
    ("c", "c"),
    ("h", "cpp"),
    ("cc", "cpp"),
    ("cpp", "cpp"),
    ("hpp", "cpp"),
];

/// File names that hold project paperwork rather than teaching material.
const PAPERWORK: [&str; 14] = [
    "license",
    "code_of_conduct",
    "code-of-conduct",
    "contributing",
    "changelog",
    "authors",
    "toc.md",
    "summary.md",
    "security.md",
    "governance",
    "maintainers",
    "owners",
    "release-notes",
    "releases",
];

/// One real section that can be taught from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The id it has in `data/train.jsonl`.
    pub id: String,
    /// The corpus source or book it belongs to, such as `trpl` or `docs`.
    pub source: String,
    /// The file it was read from, under its source.
    pub origin: String,
    /// The licence it may be used under.
    pub licence: String,
    /// The section's text.
    pub text: String,
    /// The fence tag of a source file's language; `None` for prose.
    pub language: Option<&'static str>,
}

#[derive(Deserialize)]
struct Row {
    id: String,
    response: String,
    source: String,
    origin: String,
}

/// Every book and corpus section of the training file at `train`, with the
/// licences from the corpus manifest at `manifest`.
///
/// # Errors
///
/// `DataError::Io` or `DataError::Json` when a file can't be read.
pub fn sections(train: &Path, manifest: &Path) -> Result<Vec<Section>, DataError> {
    let licences: HashMap<String, String> = corpus::read_manifest(manifest)?
        .into_iter()
        .map(|row| (row.name, row.licence.name().to_string()))
        .collect();
    let text = fs::read_to_string(train).map_err(DataError::io(train))?;
    let mut sections = Vec::new();

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let row: Row = serde_json::from_str(line)?;

        if row.source != "corpus" && row.source != "book" {
            continue;
        }

        let source = row.origin.split('/').next().unwrap_or_default().to_string();
        let licence = match row.source.as_str() {
            "corpus" => licences
                .get(&source)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
            _ => AUTHORS_OWN.to_string(),
        };
        sections.push(Section {
            id: row.id,
            source,
            origin: row.origin,
            licence,
            text: row.response,
            language: None,
        });
    }
    Ok(sections)
}

/// Every source file of the code repositories listed in `manifest` under
/// `root` whose licence allows reuse and whose size suits one lesson, in path order.
///
/// # Errors
///
/// `DataError::Io` when the manifest or a folder can't be read.
pub fn code_sections(root: &Path, manifest: &Path) -> Result<Vec<Section>, DataError> {
    let mut sections = Vec::new();

    for row in corpus::read_manifest(manifest)? {
        if row.kind != corpus::CODE_KIND || !row.licence.permits_reuse() {
            continue;
        }

        let folder = root.join(&row.name);
        let mut files = Vec::new();
        source_files(&folder, &mut files)?;
        files.sort();

        for file in files {
            let Some(language) = language_of(&file) else {
                continue;
            };

            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };

            if !(MIN_CODE_CHARS..=MAX_CODE_CHARS).contains(&text.chars().count()) {
                continue;
            }

            let relative = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .display()
                .to_string();
            sections.push(Section {
                id: stable_id(&format!("{relative}\n{text}")),
                source: row.name.clone(),
                origin: relative,
                licence: row.licence.name().to_string(),
                text,
                language: Some(language),
            });
        }
    }
    Ok(sections)
}

fn language_of(file: &Path) -> Option<&'static str> {
    let extension = file.extension()?.to_str()?;
    CODE_LANGUAGES
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|&(_, tag)| tag)
}

fn source_files(folder: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<(), DataError> {
    if !folder.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(folder)
        .map_err(DataError::io(folder))?
        .filter_map(Result::ok)
    {
        let path = entry.path();

        if path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }

        if path.is_dir() {
            source_files(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

/// True for a section worth teaching from: long enough, not paperwork, not
/// from an excluded source.
#[must_use]
pub fn is_teachable(section: &Section) -> bool {
    let file = section
        .origin
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    section.text.chars().count() >= MIN_SECTION_CHARS
        && !PAPERWORK.iter().any(|word| file.contains(word))
        && !EXCLUDED_SOURCES.contains(&section.source.as_str())
}

/// Up to `per_source` teachable sections from each source in `wanted` (every
/// source when empty), leaving out the ids in `used`. The choice is the same on
/// every run: sections are taken in order of id, which spreads them over the
/// files of a source.
#[must_use]
pub fn pick<'a>(
    sections: &'a [Section],
    per_source: usize,
    wanted: &[String],
    used: &HashSet<String>,
) -> Vec<&'a Section> {
    let mut by_source: BTreeMap<&str, Vec<&Section>> = BTreeMap::new();

    for section in sections
        .iter()
        .filter(|section| is_teachable(section) && !used.contains(&section.id))
    {
        if wanted.is_empty() || wanted.contains(&section.source) {
            by_source
                .entry(section.source.as_str())
                .or_default()
                .push(section);
        }
    }
    by_source
        .into_values()
        .flat_map(|mut found| {
            found.sort_by(|left, right| left.id.cmp(&right.id));
            found.into_iter().take(per_source)
        })
        .collect()
}

/// The picked sections as one markdown file the teacher reads.
#[must_use]
pub fn render_queue(picked: &[&Section]) -> String {
    let mut queue = String::from("# Sections to teach from\n\n");

    for section in picked {
        let body = match section.language {
            Some(language) => format!("````{language}\n{}\n````", section.text),
            None => section.text.chars().take(MAX_QUEUE_CHARS).collect(),
        };
        let _ = write!(
            queue,
            "## {}\n\n<!-- source: {}; section: {}; licence: {} -->\n\n{}\n\n",
            section.origin, section.origin, section.id, section.licence, body
        );
    }
    queue
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_files_that_are_not_text() -> Result<(), DataError> {
        let root = std::env::temp_dir().join(format!("thor-hammer-binary-{}", std::process::id()));
        let folder = root.join("lib");
        fs::create_dir_all(&folder).map_err(DataError::io(&folder))?;
        let binary = folder.join("blob.rs");
        fs::write(&binary, vec![0xff_u8; MIN_CODE_CHARS + 1]).map_err(DataError::io(&binary))?;
        let manifest = root.join("manifest.tsv");
        fs::write(
            &manifest,
            "source\tkind\tcommit\tlicence_file\tlicence\nlib\tcode\tabc\tLICENSE\tMIT License\n",
        )
        .map_err(DataError::io(&manifest))?;
        assert_eq!(code_sections(&root, &manifest)?, []);
        fs::remove_dir_all(&root).map_err(DataError::io(&root))
    }

    fn section(id: &str, source: &str, file: &str, chars: usize) -> Section {
        Section {
            id: id.to_string(),
            source: source.to_string(),
            origin: format!("{source}/{file}"),
            licence: "MIT".to_string(),
            text: "x".repeat(chars),
            language: None,
        }
    }

    #[test]
    fn leaves_out_paperwork_and_short_sections() {
        assert!(is_teachable(&section("a", "trpl", "ch08.md", 900)));
        assert!(!is_teachable(&section("a", "trpl", "ch08.md", 100)));
        assert!(!is_teachable(&section(
            "a",
            "trpl",
            "CODE_OF_CONDUCT.md",
            900
        )));
        assert!(!is_teachable(&section(
            "a",
            "gpu-accelerated-kubernetes",
            "ch01.md",
            900
        )));
    }

    #[test]
    fn reads_source_files_of_permitted_code_repositories() -> Result<(), DataError> {
        let root = std::env::temp_dir().join(format!("thor-hammer-code-{}", std::process::id()));
        let write = |path: &str, text: &str| {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap_or(&root)).map_err(DataError::io(&file))?;
            fs::write(&file, text).map_err(DataError::io(&file))
        };
        let code = "fn main() {}\n".repeat(200);
        write("lib/src/a.rs", &code)?;
        write("lib/src/notes.md", &code)?;
        write("lib/src/tiny.go", "package main\n")?;
        write("lib/.git/HEAD.rs", &code)?;
        write("docs/src/b.py", &code)?;
        write("closed/c.py", &code)?;
        let manifest = root.join("manifest.tsv");
        let rows = "source\tkind\tcommit\tlicence_file\tlicence\nlib\tcode\tabc\tLICENSE\tMIT License\ndocs\tbook\tabc\tLICENSE\tMIT License\nclosed\tcode\tabc\tLICENSE\tAll rights reserved\nmissing\tcode\tabc\tLICENSE\tMIT License\n";
        fs::write(&manifest, rows).map_err(DataError::io(&manifest))?;
        let found = code_sections(&root, &manifest)?;
        let origins: Vec<(&str, Option<&str>)> = found
            .iter()
            .map(|found| (found.origin.as_str(), found.language))
            .collect();
        assert_eq!(origins, [("lib/src/a.rs", Some("rust"))]);
        let queue = render_queue(&found.iter().collect::<Vec<_>>());
        assert!(queue.contains("````rust\nfn main() {}"));
        fs::remove_dir_all(&root).map_err(DataError::io(&root))
    }

    #[test]
    fn takes_the_same_sections_per_source_every_time() {
        let sections = [
            section("c", "go", "a.md", 900),
            section("a", "go", "b.md", 900),
            section("b", "go", "c.md", 900),
            section("z", "trpl", "d.md", 900),
        ];
        let used = HashSet::from(["a".to_string()]);
        let ids: Vec<&str> = pick(&sections, 1, &[], &used)
            .into_iter()
            .map(|picked| picked.id.as_str())
            .collect();
        assert_eq!(ids, ["b", "z"]);
        let only_go: Vec<&str> = pick(&sections, 5, &["go".to_string()], &HashSet::new())
            .into_iter()
            .map(|picked| picked.id.as_str())
            .collect();
        assert_eq!(only_go, ["a", "b", "c"]);
    }

    #[test]
    fn the_queue_carries_the_comment_an_entry_needs() {
        let picked = section("abc", "trpl", "ch08.md", 900);
        let queue = render_queue(&[&picked]);
        assert!(queue.contains("<!-- source: trpl/ch08.md; section: abc; licence: MIT -->"));
    }

    #[test]
    fn reads_sections_with_their_licences() -> Result<(), DataError> {
        let folder = std::env::temp_dir().join(format!("thor-hammer-pick-{}", std::process::id()));
        fs::create_dir_all(&folder).map_err(DataError::io(&folder))?;
        let train = folder.join("train.jsonl");
        let manifest = folder.join("manifest.tsv");
        let rows = [
            r#"{"id":"1","instruction":"","response":"text","source":"corpus","origin":"trpl/src/a.md"}"#,
            r#"{"id":"2","instruction":"","response":"text","source":"book","origin":"docs/b.md"}"#,
            r#"{"id":"3","instruction":"q","response":"a","source":"chat","origin":"c"}"#,
            r#"{"id":"4","instruction":"","response":"text","source":"corpus","origin":"gone/src/a.md"}"#,
        ];
        fs::write(&train, rows.join("\n")).map_err(DataError::io(&train))?;
        fs::write(
            &manifest,
            "source\tkind\tcommit\tlicence_file\tlicence\ntrpl\tbook\tabc\tLICENSE\tMIT License\n",
        )
        .map_err(DataError::io(&manifest))?;
        let found = sections(&train, &manifest)?;
        let licences: Vec<(&str, &str)> = found
            .iter()
            .map(|found| (found.id.as_str(), found.licence.as_str()))
            .collect();
        assert_eq!(
            licences,
            [("1", "MIT"), ("2", AUTHORS_OWN), ("4", "unknown")]
        );
        fs::remove_dir_all(&folder).map_err(DataError::io(&folder))
    }
}
