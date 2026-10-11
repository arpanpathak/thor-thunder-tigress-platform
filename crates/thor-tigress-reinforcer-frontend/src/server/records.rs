//! Records as the page draws them: one page of them, or one whole record.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::params::Params;
use crate::{
    datasets::{app::App, index::Filter, titles},
    error::{Outcome, ReviewError},
    review::flags::Flag,
    suggest::stage0::{self, Suggestions},
};

/// The most records one page may ask for.
const MAX_PAGE: usize = 100;

/// The number of records per page when the page doesn't say.
const DEFAULT_PAGE: usize = 20;

/// The longest answer sent inline with a page, in characters. A longer answer is
/// sent shortened and fetched in full only when the reader asks for it, so one
/// enormous record cannot make a whole page heavy. The longest answer in the
/// corpus is over 380,000 characters. At 6,000 most single code files arrive
/// whole, so a reviewer can read the code without expanding it.
pub(super) const MAX_INLINE_RESPONSE: usize = 6_000;

/// One page of records.
#[derive(Serialize)]
pub(super) struct Page {
    start: usize,
    total: usize,
    items: Vec<Item>,
}

pub(super) fn records(app: &App, params: &Params) -> Outcome<Page> {
    let filter = Filter {
        source: params.text("source"),
        query: params.text("q"),
        flagged: params.boolean("flagged"),
        collection: params.text("collection"),
    };
    let matches = app.index.matching(&filter, &app.flagged_ids()?)?;
    let total = matches.len();
    let start = params.number("start", 0).min(total);
    let limit = params.number("limit", DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let items = matches
        .into_iter()
        .skip(start)
        .take(limit)
        .map(|position| item(app, position, Length::ShortenedTo(MAX_INLINE_RESPONSE)))
        .collect::<Outcome<Vec<Item>>>()?;

    Ok(Page {
        start,
        total,
        items,
    })
}

/// How much of an answer to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Length {
    /// All of it.
    Whole,
    /// At most this many characters, cut at a line end.
    ShortenedTo(usize),
}

/// One record as the page draws it.
#[derive(Serialize)]
pub(super) struct Item {
    position: usize,
    id: String,
    source: String,
    source_title: String,
    collection_title: Option<String>,
    origin: String,
    instruction: String,
    response: String,
    response_chars: usize,
    shortened: bool,
    flag: Option<Flag>,
    messages: Value,
    rejected: Value,
    suggestions: Suggestions,
    source_text: Option<String>,
    licence: Option<String>,
    based_on: Option<String>,
    entry: Option<String>,
    notes: Vec<String>,
}

/// The parts of a record the page shows besides what the index holds.
#[derive(Deserialize)]
pub(super) struct RecordText {
    #[serde(default)]
    instruction: String,
    #[serde(default)]
    pub(super) response: String,
    #[serde(default)]
    messages: Value,
    #[serde(default)]
    rejected: Value,
    #[serde(default)]
    source_text: Option<String>,
    #[serde(default)]
    licence: Option<String>,
    #[serde(default)]
    based_on: Option<String>,
    #[serde(default)]
    entry: Option<String>,
    #[serde(default)]
    notes: Vec<String>,
}

/// One record, with its flag if it has one. The length of the whole answer
/// is always sent, so the reader can be told what is held back.
pub(super) fn item(app: &App, position: usize, length: Length) -> Outcome<Item> {
    let entry = app
        .index
        .entry(position)
        .ok_or_else(|| ReviewError::NotFound(format!("record {position}")))?;
    let record: RecordText = app.index.parsed(position)?;
    let source = app.index.source_of(entry);
    let response_chars = record.response.chars().count();
    let shortened = matches!(length, Length::ShortenedTo(limit) if response_chars > limit);

    let response = match length {
        Length::ShortenedTo(limit) if shortened => shorten_at_line(&record.response, limit),
        Length::ShortenedTo(_) | Length::Whole => record.response.clone(),
    };

    Ok(Item {
        position,
        id: entry.id.clone(),
        source: source.to_string(),
        source_title: titles::source_title(source),
        collection_title: app.index.collection_of(entry).map(titles::collection_title),
        origin: app.index.origin_of(entry).to_string(),
        suggestions: stage0::suggestions(source, &record.response),
        instruction: record.instruction,
        response,
        response_chars,
        shortened,
        flag: app.flags()?.get(&entry.id).cloned(),
        messages: record.messages,
        rejected: record.rejected,
        source_text: record.source_text,
        licence: record.licence,
        based_on: record.based_on,
        entry: record.entry,
        notes: record.notes,
    })
}

/// The longest run of whole lines of `text` within `limit` characters, so a
/// shortened answer never ends halfway through a line of code. Text whose first
/// line is already too long is cut at `limit` characters.
pub(super) fn shorten_at_line(text: &str, limit: usize) -> String {
    let head: String = text.chars().take(limit).collect();

    match head.rfind('\n') {
        Some(end) if end > 0 => head[..end].to_string(),
        _ => head,
    }
}

/// The examples a build removed, as the file holds them; nothing when there
/// is no such file.
pub(super) fn removed(app: &App) -> Vec<u8> {
    std::fs::read(&app.slop).unwrap_or_default()
}
