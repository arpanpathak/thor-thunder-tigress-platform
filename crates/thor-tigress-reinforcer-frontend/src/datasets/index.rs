//! A byte-offset index over the training file.
//!
//! ## Why not read the file
//!
//! The training set is tens of megabytes. Reading it into memory to answer one
//! page request would make the tool's memory grow with the corpus, which is the
//! wrong direction as the corpus grows. Instead the file is scanned once at
//! startup and only each record's position is kept: offset, length, its source
//! and its origin. That is a few megabytes for twenty thousand records and it
//! does not grow with the length of the answers.
//!
//! A page is then answered by reading exactly the bytes it shows. A search is
//! answered by streaming the file once and keeping only the positions that
//! matched, so a search costs time but not memory.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, de::DeserializeOwned};

use crate::error::{Outcome, ReviewError};

/// Read buffer for scanning the training file.
const READ_BUFFER: usize = 1 << 20;

/// Sources whose origins are `folder/file` paths, so the folder names a book.
const SOURCES_WITH_COLLECTIONS: [&str; 3] = ["book", "corpus", "teacher"];

/// The ids a set of ids is checked against: the flagged examples.
pub type Ids = HashSet<String>;

/// The fields read from a line while indexing. The answer is left out on
/// purpose: it is the large field and it is not needed to list or search a
/// record.
#[derive(Deserialize)]
struct Header<'a> {
    id: &'a str,
    source: &'a str,
    origin: &'a str,
}

/// Where one record lives, and what the list needs to draw it.
pub struct Entry {
    /// The stable example id.
    pub id: String,
    offset: u64,
    length: usize,
    source: u16,
    origin: u32,
}

/// What a page request asks for.
#[derive(Debug, Default, Clone)]
pub struct Filter {
    /// Keep only this source.
    pub source: Option<String>,
    /// Keep only records whose text contains this, case-insensitively.
    pub query: Option<String>,
    /// Keep only flagged, or only unflagged, records.
    pub flagged: Option<bool>,
    /// Keep only records read from this book or document folder.
    pub collection: Option<String>,
}

/// How many records one source holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCount {
    /// The source name.
    pub name: String,
    /// How many records.
    pub count: usize,
}

/// How many records one book or document folder holds, and in which source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CollectionCount {
    /// The folder, such as `trpl`.
    pub folder: String,
    /// The source it belongs to.
    pub source: String,
    /// How many records.
    pub count: usize,
}

/// Each distinct name stored once, by number, so a record keeps a small id
/// instead of its own copy of the text.
struct Interner<Id> {
    names: Vec<String>,
    ids: HashMap<String, Id>,
}

impl<Id> Interner<Id>
where
    Id: Copy + TryFrom<usize>,
    usize: TryFrom<Id>,
{
    fn new() -> Self {
        Interner {
            names: Vec::new(),
            ids: HashMap::new(),
        }
    }

    /// The id of `name`, adding it when it is new.
    fn id_of(&mut self, name: &str) -> Option<Id> {
        if let Some(&id) = self.ids.get(name) {
            return Some(id);
        }

        let id = Id::try_from(self.names.len()).ok()?;

        self.names.push(name.to_string());
        self.ids.insert(name.to_string(), id);

        Some(id)
    }

    fn name(&self, id: Id) -> &str {
        usize::try_from(id)
            .ok()
            .and_then(|index| self.names.get(index))
            .map_or("", String::as_str)
    }
}

/// The training file, addressed by byte offset.
pub struct Index {
    file: File,
    path: PathBuf,
    entries: Vec<Entry>,
    ids: Ids,
    sources: Interner<u16>,
    origins: Interner<u32>,
}

impl Index {
    /// Scans `path` once and builds the index.
    ///
    /// # Errors
    ///
    /// `ReviewError::Io` for an unreadable file, `ReviewError::Json` for a
    /// line without `id`, `source` and `origin`, `ReviewError::BadRequest` for
    /// more distinct sources or origins than the index can number.
    pub fn open(path: &Path) -> Outcome<Self> {
        let file = File::open(path).map_err(ReviewError::io(path))?;
        let mut reader = BufReader::with_capacity(
            READ_BUFFER,
            file.try_clone().map_err(ReviewError::io(path))?,
        );
        let mut index = Index {
            file,
            path: path.to_path_buf(),
            entries: Vec::new(),
            ids: HashSet::new(),
            sources: Interner::new(),
            origins: Interner::new(),
        };
        let mut offset = 0;
        let mut line = String::new();

        for line_number in 1.. {
            line.clear();

            let read = reader.read_line(&mut line).map_err(ReviewError::io(path))?;

            if read == 0 {
                break;
            }

            if !line.trim().is_empty() {
                let header: Header =
                    serde_json::from_str(&line).map_err(ReviewError::json(path, line_number))?;
                index.add(&header, offset, read)?;
            }

            offset += u64::try_from(read).unwrap_or(u64::MAX);
        }

        Ok(index)
    }

    fn add(&mut self, header: &Header, offset: u64, length: usize) -> Outcome {
        let too_many =
            || ReviewError::BadRequest("too many distinct sources or origins".to_string());
        let entry = Entry {
            id: header.id.to_string(),
            offset,
            length,
            source: self.sources.id_of(header.source).ok_or_else(too_many)?,
            origin: self.origins.id_of(header.origin).ok_or_else(too_many)?,
        };
        self.ids.insert(entry.id.clone());
        self.entries.push(entry);

        Ok(())
    }

    /// The number of records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when a record with `id` is in the index.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// The file this index describes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The source name of one record.
    #[must_use]
    pub fn source_of(&self, entry: &Entry) -> &str {
        self.sources.name(entry.source)
    }

    /// The origin of one record.
    #[must_use]
    pub fn origin_of(&self, entry: &Entry) -> &str {
        self.origins.name(entry.origin)
    }

    /// Every entry, in file order.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    /// The entry at `position`.
    #[must_use]
    pub fn entry(&self, position: usize) -> Option<&Entry> {
        self.entries.get(position)
    }

    /// The book or document folder a record was read from: the first part of
    /// its origin, such as `trpl` for `trpl/src/ch01.md`. Records from the chat
    /// export and the reviewer's own files have none.
    #[must_use]
    pub fn collection_of(&self, entry: &Entry) -> Option<&str> {
        if !SOURCES_WITH_COLLECTIONS.contains(&self.source_of(entry)) {
            return None;
        }

        let (folder, _) = self.origin_of(entry).split_once('/')?;

        (!folder.is_empty()).then_some(folder)
    }

    /// Every book or document folder with its source and record count, most
    /// records first.
    #[must_use]
    pub fn collections(&self) -> Vec<CollectionCount> {
        let mut counts: HashMap<(&str, &str), usize> = HashMap::new();

        for entry in &self.entries {
            if let Some(folder) = self.collection_of(entry) {
                *counts.entry((folder, self.source_of(entry))).or_default() += 1;
            }
        }

        let mut collections: Vec<CollectionCount> = counts
            .into_iter()
            .map(|((folder, source), count)| CollectionCount {
                folder: folder.to_string(),
                source: source.to_string(),
                count,
            })
            .collect();

        collections.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.folder.cmp(&right.folder))
        });

        collections
    }

    /// How many records each source holds, most first.
    #[must_use]
    pub fn counts(&self) -> Vec<SourceCount> {
        let mut counts: HashMap<&str, usize> = HashMap::new();

        for entry in &self.entries {
            *counts.entry(self.source_of(entry)).or_default() += 1;
        }

        let mut sources: Vec<SourceCount> = counts
            .into_iter()
            .map(|(name, count)| SourceCount {
                name: name.to_string(),
                count,
            })
            .collect();

        sources.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.name.cmp(&right.name))
        });

        sources
    }

    /// Reads one whole record from disk, as the JSON line it is.
    ///
    /// # Errors
    ///
    /// `ReviewError::NotFound` for a position past the end, `ReviewError::Io`
    /// when the read fails, `ReviewError::BadRequest` for text that isn't UTF-8.
    pub fn record(&self, position: usize) -> Outcome<String> {
        let entry = self
            .entries
            .get(position)
            .ok_or_else(|| ReviewError::NotFound(format!("record {position}")))?;
        let mut buffer = vec![0; entry.length];

        self.file
            .read_exact_at(&mut buffer, entry.offset)
            .map_err(ReviewError::io(&self.path))?;

        String::from_utf8(buffer)
            .map_err(|_| ReviewError::BadRequest(format!("record {position} is not UTF-8")))
    }

    /// One record, read as a `T`.
    ///
    /// # Errors
    ///
    /// As for [`Index::record`], and `ReviewError::Json` when the record isn't a `T`.
    pub fn parsed<T: DeserializeOwned>(&self, position: usize) -> Outcome<T> {
        serde_json::from_str(&self.record(position)?)
            .map_err(ReviewError::json(&self.path, position + 1))
    }

    /// Every position the filter selects, in file order.
    ///
    /// A filter with no query is answered from the index. A filter with a query
    /// streams the file once and keeps only the positions that matched, so the
    /// memory cost is the number of matches rather than the size of the corpus.
    ///
    /// # Errors
    ///
    /// `ReviewError::Io` when the file can't be read for a search.
    pub fn matching(&self, filter: &Filter, flagged: &Ids) -> Outcome<Vec<usize>> {
        let kept = |position: &usize| {
            self.entries
                .get(*position)
                .is_some_and(|entry| self.keeps(entry, filter, flagged))
        };

        match filter.query.as_deref() {
            None => Ok((0..self.entries.len()).filter(kept).collect()),
            Some(query) => Ok(self.containing(query)?.into_iter().filter(kept).collect()),
        }
    }

    /// The positions of records whose line contains `query`, case-insensitively.
    fn containing(&self, query: &str) -> Outcome<Vec<usize>> {
        let needle = query.to_lowercase();
        let escaped = json_escaped(&needle);
        let file = File::open(&self.path).map_err(ReviewError::io(&self.path))?;
        let mut positions = Vec::new();
        let mut position = 0;

        for line in BufReader::with_capacity(READ_BUFFER, file).lines() {
            let line = line.map_err(ReviewError::io(&self.path))?.to_lowercase();

            if line.trim().is_empty() {
                continue;
            }

            if line.contains(&needle) || line.contains(&escaped) {
                positions.push(position);
            }

            position += 1;
        }

        Ok(positions)
    }

    /// True when one entry passes the source, flag and collection parts of a filter.
    fn keeps(&self, entry: &Entry, filter: &Filter, flagged: &Ids) -> bool {
        let source_ok = filter
            .source
            .as_deref()
            .is_none_or(|source| self.source_of(entry) == source);
        let flag_ok = filter
            .flagged
            .is_none_or(|wanted| wanted == flagged.contains(&entry.id));
        let collection_ok = filter
            .collection
            .as_deref()
            .is_none_or(|wanted| self.collection_of(entry) == Some(wanted));

        source_ok && flag_ok && collection_ok
    }
}

/// `text` as it appears inside a JSON string, so a query with a quote or a
/// backslash still matches the raw record line.
fn json_escaped(text: &str) -> String {
    let quoted = serde_json::Value::from(text).to_string();
    let inner = quoted
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'));

    inner.unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests;
