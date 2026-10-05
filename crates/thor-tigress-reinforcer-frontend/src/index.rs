//! A byte-offset index over the training file.
//!
//! ## Why not read the file
//!
//! The training set is tens of megabytes. Reading it into memory to answer one
//! page request would make the tool's memory grow with the corpus, which is the
//! wrong direction as the corpus grows. Instead the file is scanned once at
//! startup and only each record's position is kept: offset, length, its source,
//! its origin and the request text. That is a few megabytes for twenty thousand
//! records and it does not grow with the length of the answers.
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

use serde::Deserialize;

use crate::error::ReviewError;

/// The fields read from a line while indexing. The answer is left out on
/// purpose: it is the large field and it is not needed to list or search a
/// record.
#[derive(Deserialize)]
struct Header<'a> {
    /// The stable example id.
    id: &'a str,
    /// Which input the example came from.
    source: &'a str,
    /// The file or conversation it came from.
    origin: &'a str,
}

/// Where one record lives, and what the list needs to draw it.
pub struct Entry {
    /// The stable example id.
    pub id: String,
    /// Byte offset of the line.
    pub offset: u64,
    /// Byte length of the line, newline included.
    pub length: u32,
    /// Index into [`Index::sources`].
    pub source: u16,
    /// Index into [`Index::origins`].
    pub origin: u32,
}

/// What a page request asks for.
#[derive(Default, Clone)]
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

impl Filter {
    /// True when the filter can be answered from the index alone.
    fn is_index_only(&self) -> bool {
        self.query.is_none()
    }
}

/// The training file, addressed by byte offset.
pub struct Index {
    file: File,
    path: PathBuf,
    entries: Vec<Entry>,
    ids: HashSet<String>,
    sources: Vec<String>,
    origins: Vec<String>,
}

impl Index {
    /// Scans `path` once and builds the index.
    pub fn open(path: &Path) -> Result<Self, ReviewError> {
        let file = File::open(path).map_err(ReviewError::io(path))?;
        let reader = BufReader::with_capacity(1 << 20, file.try_clone().map_err(ReviewError::io(path))?);
        let mut entries = Vec::new();
        let mut sources = Vec::new();
        let mut origins = Vec::new();
        let mut source_ids: HashMap<String, u16> = HashMap::new();
        let mut origin_ids: HashMap<String, u32> = HashMap::new();
        let mut offset = 0u64;
        let mut line = String::new();
        let mut line_number = 0usize;
        let mut reader = reader;
        loop {
            line.clear();
            let read = reader.read_line(&mut line).map_err(ReviewError::io(path))?;
            if read == 0 {
                break;
            }
            line_number += 1;
            if !line.trim().is_empty() {
                let header: Header =
                    serde_json::from_str(&line).map_err(ReviewError::json(path, line_number))?;
                entries.push(Entry {
                    id: header.id.to_string(),
                    offset,
                    length: u32::try_from(read).unwrap_or(u32::MAX),
                    source: intern_source(&mut sources, &mut source_ids, header.source),
                    origin: intern_origin(&mut origins, &mut origin_ids, header.origin),
                });
            }
            offset += read as u64;
        }
        Ok(Index {
            file,
            path: path.to_path_buf(),
            ids: entries.iter().map(|entry| entry.id.clone()).collect(),
            entries,
            sources,
            origins,
        })
    }

    /// The number of records.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when a record with `id` is in the index.
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// The file this index describes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The source name of one record.
    pub fn source_of(&self, entry: &Entry) -> &str {
        self.sources
            .get(entry.source as usize)
            .map(String::as_str)
            .unwrap_or_default()
    }

    /// The origin of one record.
    pub fn origin_of(&self, entry: &Entry) -> &str {
        self.origins
            .get(entry.origin as usize)
            .map(String::as_str)
            .unwrap_or_default()
    }

    /// Every entry, in file order.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    /// The book or document folder a record was read from: the first part of
    /// its origin, such as `trpl` for `trpl/src/ch01.md`. Records from the chat
    /// export and the reviewer's own files have none.
    pub fn collection_of(&self, entry: &Entry) -> Option<&str> {
        let has_collections = matches!(self.source_of(entry), "book" | "corpus");
        let origin = self.origin_of(entry);
        has_collections
            .then(|| origin.split('/').next())
            .flatten()
            .filter(|folder| !folder.is_empty() && origin.contains('/'))
    }

    /// Every book or document folder with its source and record count, most
    /// records first.
    pub fn collections(&self) -> Vec<(String, String, usize)> {
        let mut counts: HashMap<(String, String), usize> = HashMap::new();
        for entry in &self.entries {
            if let Some(folder) = self.collection_of(entry) {
                *counts
                    .entry((folder.to_string(), self.source_of(entry).to_string()))
                    .or_default() += 1;
            }
        }
        let mut collections: Vec<(String, String, usize)> = counts
            .into_iter()
            .map(|((folder, source), count)| (folder, source, count))
            .collect();
        collections.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));
        collections
    }

    /// The entry at `position`.
    pub fn entry(&self, position: usize) -> Option<&Entry> {
        self.entries.get(position)
    }

    /// How many records each source holds, most first.
    pub fn counts(&self) -> Vec<(String, usize)> {
        let mut counts = vec![0usize; self.sources.len()];
        for entry in &self.entries {
            if let Some(slot) = counts.get_mut(entry.source as usize) {
                *slot += 1;
            }
        }
        let mut pairs: Vec<(String, usize)> = self
            .sources
            .iter()
            .cloned()
            .zip(counts)
            .filter(|(_, count)| *count > 0)
            .collect();
        pairs.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        pairs
    }

    /// Reads one whole record from disk, as the JSON line it is.
    pub fn record(&self, position: usize) -> Result<String, ReviewError> {
        let entry = self
            .entries
            .get(position)
            .ok_or_else(|| ReviewError::NotFound(format!("record {position}")))?;
        let mut buffer = vec![0u8; entry.length as usize];
        self.file
            .read_exact_at(&mut buffer, entry.offset)
            .map_err(ReviewError::io(&self.path))?;
        String::from_utf8(buffer)
            .map_err(|_| ReviewError::BadRequest(format!("record {position} is not UTF-8")))
    }

    /// Every position the filter selects, in file order.
    ///
    /// A filter with no query is answered from the index. A filter with a query
    /// streams the file once and keeps only the positions that matched, so the
    /// memory cost is the number of matches rather than the size of the corpus.
    pub fn matching(
        &self,
        filter: &Filter,
        flagged_ids: &HashSet<String>,
    ) -> Result<Vec<usize>, ReviewError> {
        match filter.is_index_only() {
            true => Ok(self.matching_in_memory(filter, flagged_ids)),
            false => self.matching_by_scanning(filter, flagged_ids),
        }
    }

    /// The positions a filter with no query selects.
    fn matching_in_memory(&self, filter: &Filter, flagged_ids: &HashSet<String>) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.keeps(entry, filter, flagged_ids))
            .map(|(position, _)| position)
            .collect()
    }

    /// The positions a filter with a query selects, by streaming the file.
    fn matching_by_scanning(
        &self,
        filter: &Filter,
        flagged_ids: &HashSet<String>,
    ) -> Result<Vec<usize>, ReviewError> {
        let needle = filter
            .query
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();
        let escaped = json_escaped(&needle);
        let file = File::open(&self.path).map_err(ReviewError::io(&self.path))?;
        let reader = BufReader::with_capacity(1 << 20, file);
        let mut positions = Vec::new();
        let mut position = 0usize;
        for line in reader.lines() {
            let text = line.map_err(ReviewError::io(&self.path))?;
            if text.trim().is_empty() {
                continue;
            }
            let lowered = text.to_lowercase();
            let matches_query = lowered.contains(&needle) || lowered.contains(&escaped);
            let keeps = self
                .entries
                .get(position)
                .is_some_and(|entry| self.keeps(entry, filter, flagged_ids));
            if matches_query && keeps {
                positions.push(position);
            }
            position += 1;
        }
        Ok(positions)
    }

    /// True when one entry passes the source and flag parts of a filter.
    fn keeps(&self, entry: &Entry, filter: &Filter, flagged_ids: &HashSet<String>) -> bool {
        let source_ok = filter
            .source
            .as_deref()
            .is_none_or(|source| self.source_of(entry) == source);
        let is_flagged = flagged_ids.contains(&entry.id);
        let flag_ok = filter.flagged.is_none_or(|wanted| wanted == is_flagged);
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
    let quoted = serde_json::to_string(text).unwrap_or_default();
    quoted
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or_default()
        .to_string()
}

/// The id of a source name, adding it to the table when it is new.
fn intern_source(table: &mut Vec<String>, ids: &mut HashMap<String, u16>, text: &str) -> u16 {
    match ids.get(text) {
        Some(id) => *id,
        None => {
            let id = u16::try_from(table.len()).unwrap_or(u16::MAX);
            table.push(text.to_string());
            ids.insert(text.to_string(), id);
            id
        }
    }
}

/// The id of an origin, adding it to the table when it is new.
fn intern_origin(table: &mut Vec<String>, ids: &mut HashMap<String, u32>, text: &str) -> u32 {
    match ids.get(text) {
        Some(id) => *id,
        None => {
            let id = u32::try_from(table.len()).unwrap_or(u32::MAX);
            table.push(text.to_string());
            ids.insert(text.to_string(), id);
            id
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An index over a temporary file holding `lines`.
    fn index_over(name: &str, lines: &str) -> Result<(Index, PathBuf), ReviewError> {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, lines).map_err(ReviewError::io(&path))?;
        Ok((Index::open(&path)?, path))
    }

    const LINES: &str = concat!(
        r#"{"id":"a1","instruction":"What is a page?","response":"A block.","source":"book","origin":"ch01.md"}"#,
        "\n",
        r#"{"id":"b2","instruction":"What is a cache?","response":"Fast memory.","source":"chat","origin":"c1"}"#,
        "\n",
        r#"{"id":"c3","instruction":"What is a page table?","response":"Maps pages.","source":"book","origin":"ch02.md"}"#,
        "\n"
    );

    #[test]
    fn indexes_every_record_with_its_source() -> Result<(), ReviewError> {
        let (index, path) = index_over("flagger_index_a.jsonl", LINES)?;
        assert_eq!(index.len(), 3);
        assert_eq!(index.counts(), [("book".to_string(), 2), ("chat".to_string(), 1)]);
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn reads_a_record_back_whole() -> Result<(), ReviewError> {
        let (index, path) = index_over("flagger_index_b.jsonl", LINES)?;
        let record = index.record(1)?;
        assert!(record.contains("Fast memory."));
        assert!(record.starts_with(r#"{"id":"b2""#));
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn filters_by_source_without_reading_the_file() -> Result<(), ReviewError> {
        let (index, path) = index_over("flagger_index_c.jsonl", LINES)?;
        let filter = Filter {
            source: Some("book".to_string()),
            ..Filter::default()
        };
        let matches = index.matching(&filter, &HashSet::new())?;
        assert_eq!(matches, [0, 2]);
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn searches_the_whole_record() -> Result<(), ReviewError> {
        let (index, path) = index_over("flagger_index_d.jsonl", LINES)?;
        let filter = Filter {
            query: Some("page table".to_string()),
            ..Filter::default()
        };
        let matches = index.matching(&filter, &HashSet::new())?;
        assert_eq!(matches, [2]);
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn finds_a_query_that_contains_a_quote() -> Result<(), ReviewError> {
        let lines = concat!(r#"{"id":"q1","instruction":"x","response":"say \"hi\" now","source":"chat","origin":"c"}"#, "\n");
        let (index, path) = index_over("flagger_index_quote.jsonl", lines)?;
        let filter = Filter {
            query: Some("say \"hi\"".to_string()),
            ..Filter::default()
        };
        assert_eq!(index.matching(&filter, &HashSet::new())?, [0]);
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn filters_by_the_book_a_record_came_from() -> Result<(), ReviewError> {
        let (index, path) = index_over("flagger_index_f.jsonl", LINES)?;
        assert_eq!(index.collections(), []);
        let lines = concat!(
            r#"{"id":"t1","instruction":"a","response":"b","source":"corpus","origin":"trpl/src/ch01.md"}"#, "\n",
            r#"{"id":"r1","instruction":"a","response":"b","source":"corpus","origin":"rbe/src/hello.md"}"#, "\n",
            r#"{"id":"c1","instruction":"a","response":"b","source":"chat","origin":"conversations.json#x/1"}"#, "\n"
        );
        std::fs::remove_file(path).ok();
        let (index, path) = index_over("flagger_index_g.jsonl", lines)?;
        let filter = Filter {
            collection: Some("trpl".to_string()),
            ..Filter::default()
        };
        assert_eq!(index.matching(&filter, &HashSet::new())?, [0]);
        assert_eq!(index.collections().len(), 2);
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn keeps_only_flagged_records_when_asked() -> Result<(), ReviewError> {
        let (index, path) = index_over("flagger_index_e.jsonl", LINES)?;
        let flagged: HashSet<String> = ["b2".to_string()].into_iter().collect();
        let filter = Filter {
            flagged: Some(true),
            ..Filter::default()
        };
        let matches = index.matching(&filter, &flagged)?;
        assert_eq!(matches, [1]);
        std::fs::remove_file(path).ok();
        Ok(())
    }
}
