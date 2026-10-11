//! The reviewer's flags: setting one, marking a phrase everywhere, and the
//! flags a rebuild left without an example.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::records::RecordText;
use crate::{
    datasets::{
        app::App,
        index::{Filter, Ids},
    },
    review::{
        category::{Field, SlopCategory},
        flags::{Flag, MIN_PHRASE_CHARS, Phrase, Span},
    },
    error::{Outcome, ReviewError},
};

/// How a flag set by marking a phrase everywhere begins its note.
const PHRASE_NOTE: &str = "phrase:";

/// Flags whose example is in neither the training set nor the removed examples.
#[derive(Serialize)]
pub(super) struct Orphans {
    orphans: Vec<Orphan>,
}

#[derive(Serialize)]
pub(super) struct Orphan {
    id: String,
    note: String,
    spans: Vec<Span>,
}

/// A rebuild that changed an example's text leaves its flag here: the id is a
/// hash of the text, so it can no longer find its example. The page shows
/// these so a person can re-point them instead of losing them. Untouched
/// machine suggestions are left out.
pub(super) fn orphans(app: &App) -> Outcome<Orphans> {
    let store = app.flags()?;
    let mut orphans: Vec<Orphan> = store
        .iter()
        .filter(|(id, flag)| {
            !app.index.contains(id) && !app.slop_ids.contains(*id) && flag.is_reviewed()
        })
        .map(|(id, flag)| Orphan {
            id: id.to_string(),
            note: flag.note.clone(),
            spans: flag.spans.clone(),
        })
        .collect();
    orphans.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Orphans { orphans })
}

/// What a request to change a flag carries.
#[derive(Deserialize)]
pub(super) struct FlagRequest {
    id: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    spans: Vec<Span>,
    #[serde(default = "flag_by_default")]
    flagged: bool,
}

/// A body that doesn't say `flagged` sets a flag.
fn flag_by_default() -> bool {
    true
}

#[derive(Serialize)]
pub(super) struct FlagSaved {
    ok: bool,
    flagged: bool,
}

/// Sets or clears one flag, and writes the file.
pub(super) fn set_flag(app: &App, body: &str) -> Outcome<FlagSaved> {
    let wanted: FlagRequest = parsed(body, "flag")?;
    let mut store = app.flags()?;

    if wanted.flagged {
        store.set(
            &wanted.id,
            Flag {
                note: wanted.note,
                spans: wanted.spans,
            },
        );
    } else {
        store.clear(&wanted.id);
    }

    store.save()?;

    Ok(FlagSaved {
        ok: true,
        flagged: wanted.flagged,
    })
}

/// Every phrase a reviewer has marked, for the page to highlight everywhere.
#[derive(Serialize)]
pub(super) struct Phrases {
    pub(super) phrases: Vec<Phrase>,
}

/// What a request to mark a phrase everywhere carries.
#[derive(Deserialize)]
pub(super) struct MatchRequest {
    text: String,
    category: SlopCategory,
}

#[derive(Serialize)]
pub(super) struct Matched {
    matched: usize,
    added: usize,
}

/// The phrase as it is written in one answer.
pub(super) struct Occurrence {
    id: String,
    text: String,
}

/// Marks a phrase in the answer of every example that contains it, ignoring
/// case. Examples that already carry the phrase are left as they are.
pub(super) fn flag_matches(app: &App, body: &str) -> Outcome<Matched> {
    let wanted: MatchRequest = parsed(body, "flag-matches")?;
    let needle = wanted.text.trim().to_lowercase();

    if needle.chars().count() < MIN_PHRASE_CHARS {
        return Err(ReviewError::BadRequest(format!(
            "a phrase needs at least {MIN_PHRASE_CHARS} characters"
        )));
    }

    let filter = Filter {
        query: Some(needle.clone()),
        ..Filter::default()
    };
    let mut found = Vec::new();

    for position in app.index.matching(&filter, &Ids::new())? {
        let record: RecordText = app.index.parsed(position)?;

        let Some(text) = occurrence(&record.response, &needle) else {
            continue;
        };

        let id = app
            .index
            .entry(position)
            .map(|entry| entry.id.clone())
            .unwrap_or_default();
        found.push(Occurrence { id, text });
    }

    let mut store = app.flags()?;
    let note = format!("{PHRASE_NOTE} {}", wanted.text.trim());
    let added = found
        .iter()
        .filter(|occurrence| {
            let span = Span {
                field: Field::Response,
                text: occurrence.text.clone(),
                category: wanted.category,
            };

            store.add_span(&occurrence.id, span, &note)
        })
        .count();

    store.save()?;

    Ok(Matched {
        matched: found.len(),
        added,
    })
}

/// The first place `needle` (lowercase) appears in `text`, as written there.
fn occurrence(text: &str, needle: &str) -> Option<String> {
    let start = text.to_lowercase().find(needle)?;

    text.get(start..start + needle.len()).map(str::to_string)
}

/// A request's JSON body as a `T`; `what` names it in the error.
fn parsed<T: DeserializeOwned>(body: &str, what: &str) -> Outcome<T> {
    serde_json::from_str(body)
        .map_err(|error| ReviewError::BadRequest(format!("{what} body: {error}")))
}
