//! The review page's API: one `Route` per thing the page asks for, one small
//! function that answers each, and the shapes of the answers.

use std::{fs, io::Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    app::App,
    category::{Field, SlopCategory},
    error::{Outcome, ReviewError},
    flags::{Flag, MIN_PHRASE_CHARS, Phrase, Span},
    http::{self, ContentType, Request, Status},
    index::{Filter, Ids},
    stage0::{self, Suggestions},
    titles,
    workspace::{DatasetInfo, Workspace},
};

/// The page, compiled into the binary.
const PAGE: &str = include_str!("page.html");

/// The most records one page may ask for.
const MAX_PAGE: usize = 100;

/// The number of records per page when the page doesn't say.
const DEFAULT_PAGE: usize = 20;

/// The longest answer sent inline with a page, in characters. A longer answer is
/// sent shortened and fetched in full only when the reader asks for it, so one
/// enormous record cannot make a whole page heavy. The longest answer in the
/// corpus is over 380,000 characters. At 6,000 most single code files arrive
/// whole, so a reviewer can read the code without expanding it.
const MAX_INLINE_RESPONSE: usize = 6_000;

/// How a flag set by marking a phrase everywhere begins its note.
const PHRASE_NOTE: &str = "phrase:";

/// What a request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The review page.
    Page,
    /// The datasets the page can switch between.
    Datasets,
    /// Counts for the header.
    Meta,
    /// Flags whose example is gone.
    Orphans,
    /// The examples a build removed, as they are.
    SlopFile,
    /// The categories the picker offers.
    Categories,
    /// One page of records.
    Records,
    /// One whole record.
    Record,
    /// Set or clear one flag.
    SetFlag,
    /// Every marked phrase.
    Phrases,
    /// Mark a phrase in every example that contains it.
    FlagMatches,
    /// Anything else.
    NotFound,
}

impl Route {
    /// The route for `method` and `path`.
    #[must_use]
    pub fn of(method: &str, path: &str) -> Route {
        match (method, path) {
            ("GET", "/") => Route::Page,
            ("GET", "/api/datasets") => Route::Datasets,
            ("GET", "/api/meta") => Route::Meta,
            ("GET", "/api/orphans") => Route::Orphans,
            ("GET", "/slop.jsonl") => Route::SlopFile,
            ("GET", "/api/categories") => Route::Categories,
            ("GET", "/api/page") => Route::Records,
            ("GET", "/api/record") => Route::Record,
            ("POST", "/api/flag") => Route::SetFlag,
            ("GET", "/api/phrases") => Route::Phrases,
            ("POST", "/api/flag-matches") => Route::FlagMatches,
            _ => Route::NotFound,
        }
    }
}

/// What the page's dataset switcher shows.
#[derive(Serialize)]
struct Datasets {
    datasets: Vec<DatasetInfo>,
    missing: Vec<String>,
}

/// Answers one request. Every route except the page and the dataset list
/// works on the dataset named by `?dataset=`, the first one when not named.
///
/// # Errors
///
/// `ReviewError::NotFound` for an unknown route or dataset, and whatever the
/// route's handler returns.
pub fn answer(stream: &mut dyn Write, request: &Request, workspace: &Workspace) -> Outcome {
    match Route::of(&request.method, &request.path) {
        Route::Page => http::write_response(stream, Status::Ok, ContentType::Html, PAGE.as_bytes()),
        Route::Datasets => ok(
            stream,
            &Datasets {
                datasets: workspace.infos()?,
                missing: workspace.missing().to_vec(),
            },
        ),
        _ => answer_in(
            stream,
            request,
            &workspace.dataset(request.param("dataset"))?.app,
        ),
    }
}

/// Answers one request about one dataset.
fn answer_in(stream: &mut dyn Write, request: &Request, app: &App) -> Outcome {
    match Route::of(&request.method, &request.path) {
        Route::Page | Route::Datasets => Err(ReviewError::NotFound(format!(
            "{} {}",
            request.method, request.path
        ))),
        Route::SlopFile => {
            let removed = fs::read(&app.slop).unwrap_or_default();
            http::write_response(stream, Status::Ok, ContentType::JsonLines, &removed)
        }
        Route::Meta => ok(stream, &meta(app)?),
        Route::Orphans => ok(stream, &orphans(app)?),
        Route::Categories => ok(stream, &categories()),
        Route::Records => ok(stream, &records(app, request)?),
        Route::Record => ok(
            stream,
            &item(app, request.number("position", usize::MAX), Length::Whole)?,
        ),
        Route::SetFlag => ok(stream, &set_flag(app, request)?),
        Route::Phrases => ok(
            stream,
            &Phrases {
                phrases: app.flags()?.phrases(),
            },
        ),
        Route::FlagMatches => ok(stream, &flag_matches(app, request)?),
        Route::NotFound => Err(ReviewError::NotFound(format!(
            "{} {}",
            request.method, request.path
        ))),
    }
}

fn ok(stream: &mut dyn Write, value: &impl Serialize) -> Outcome {
    http::write_json(stream, Status::Ok, value)
}

/// What the header shows.
#[derive(Serialize)]
struct Meta {
    total: usize,
    flagged: usize,
    sources: Vec<SourceInfo>,
    collections: Vec<CollectionInfo>,
    file: String,
}

#[derive(Serialize)]
struct SourceInfo {
    name: String,
    title: String,
    count: usize,
}

#[derive(Serialize)]
struct CollectionInfo {
    name: String,
    title: String,
    source: String,
    count: usize,
}

fn meta(app: &App) -> Outcome<Meta> {
    let sources = app
        .index
        .counts()
        .into_iter()
        .map(|source| SourceInfo {
            title: titles::source_title(&source.name),
            name: source.name,
            count: source.count,
        })
        .collect();
    let collections = app
        .index
        .collections()
        .into_iter()
        .map(|collection| CollectionInfo {
            title: titles::collection_title(&collection.folder),
            name: collection.folder,
            source: collection.source,
            count: collection.count,
        })
        .collect();
    Ok(Meta {
        total: app.index.len(),
        flagged: app.flags()?.len(),
        sources,
        collections,
        file: app.index.path().display().to_string(),
    })
}

/// Flags whose example is in neither the training set nor the removed examples.
#[derive(Serialize)]
struct Orphans {
    orphans: Vec<Orphan>,
}

#[derive(Serialize)]
struct Orphan {
    id: String,
    note: String,
    spans: Vec<Span>,
}

/// A rebuild that changed an example's text leaves its flag here: the id is a
/// hash of the text, so it can no longer find its example. The page shows
/// these so a person can re-point them instead of losing them. Untouched
/// machine suggestions are left out.
fn orphans(app: &App) -> Outcome<Orphans> {
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

/// The categories the picker offers.
#[derive(Serialize)]
struct Categories {
    categories: Vec<CategoryInfo>,
}

#[derive(Serialize)]
struct CategoryInfo {
    name: SlopCategory,
    description: &'static str,
}

fn categories() -> Categories {
    let categories = SlopCategory::ALL
        .into_iter()
        .map(|category| CategoryInfo {
            name: category,
            description: category.description(),
        })
        .collect();
    Categories { categories }
}

/// One page of records.
#[derive(Serialize)]
struct Page {
    start: usize,
    total: usize,
    items: Vec<Item>,
}

fn records(app: &App, request: &Request) -> Outcome<Page> {
    let filter = Filter {
        source: request.text("source"),
        query: request.text("q"),
        flagged: request.boolean("flagged"),
        collection: request.text("collection"),
    };
    let matches = app.index.matching(&filter, &app.flagged_ids()?)?;
    let total = matches.len();
    let start = request.number("start", 0).min(total);
    let limit = request.number("limit", DEFAULT_PAGE).clamp(1, MAX_PAGE);
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
enum Length {
    /// All of it.
    Whole,
    /// At most this many characters, cut at a line end.
    ShortenedTo(usize),
}

/// One record as the page draws it.
#[derive(Serialize)]
struct Item {
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
struct RecordText {
    #[serde(default)]
    instruction: String,
    #[serde(default)]
    response: String,
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
fn item(app: &App, position: usize, length: Length) -> Outcome<Item> {
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
fn shorten_at_line(text: &str, limit: usize) -> String {
    let head: String = text.chars().take(limit).collect();

    match head.rfind('\n') {
        Some(end) if end > 0 => head[..end].to_string(),
        _ => head,
    }
}

/// What a request to change a flag carries.
#[derive(Deserialize)]
struct FlagRequest {
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
struct FlagSaved {
    ok: bool,
    flagged: bool,
}

/// Sets or clears one flag, and writes the file.
fn set_flag(app: &App, request: &Request) -> Outcome<FlagSaved> {
    let wanted: FlagRequest = body(request, "flag")?;
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
struct Phrases {
    phrases: Vec<Phrase>,
}

/// What a request to mark a phrase everywhere carries.
#[derive(Deserialize)]
struct MatchRequest {
    text: String,
    category: SlopCategory,
}

#[derive(Serialize)]
struct Matched {
    matched: usize,
    added: usize,
}

/// The phrase as it is written in one answer.
struct Occurrence {
    id: String,
    text: String,
}

/// Marks a phrase in the answer of every example that contains it, ignoring
/// case. Examples that already carry the phrase are left as they are.
fn flag_matches(app: &App, request: &Request) -> Outcome<Matched> {
    let wanted: MatchRequest = body(request, "flag-matches")?;
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
fn body<T: serde::de::DeserializeOwned>(request: &Request, what: &str) -> Outcome<T> {
    serde_json::from_str(&request.body)
        .map_err(|error| ReviewError::BadRequest(format!("{what} body: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn a_phrase_found_outside_the_answers_marks_nothing() -> Outcome {
        let setup = setup()?;
        let reply = call(
            &setup.app,
            "POST",
            "/api/flag-matches",
            r#"{"text":"trpl/src","category":"other"}"#,
        )?;
        assert_eq!(
            (reply.body["matched"].as_u64(), reply.body["added"].as_u64()),
            (Some(0), Some(0))
        );
        Ok(())
    }

    const TRAINING: &str = concat!(
        r#"{"id":"a1","source":"chat","origin":"c1","instruction":"Why?","response":"Great question! Here's the thing: no."}"#,
        "\n",
        r#"{"id":"b2","source":"corpus","origin":"trpl/src/ch01.md","instruction":"What is a page?","response":"Here's The Thing about pages."}"#,
        "\n",
        r#"{"id":"c3","source":"readability","origin":"r","instruction":"Rewrite","response":"line one\nline two\nline three"}"#,
        "\n"
    );

    struct Setup {
        app: App,
        _folder: TempDir,
    }

    fn setup() -> Outcome<Setup> {
        let folder = TempDir::new()?;
        let training = folder.file("train.jsonl", TRAINING)?;
        let slop = folder.file("slop.jsonl", "{\"id\":\"gone\",\"response\":\"old\"}\n")?;
        let flag_lines = concat!(
            r#"{"id":"gone","note":"removed"}"#,
            "\n",
            r#"{"id":"lost","note":"text changed","spans":[]}"#,
            "\n",
            r#"{"id":"also-lost","note":"rebuilt","spans":[]}"#,
            "\n",
            r#"{"id":"auto-only","note":"auto: x","spans":[]}"#,
            "\n"
        );
        let flags = folder.file("flags.jsonl", flag_lines)?;
        let app = App::open(&training, &flags, &slop)?;
        Ok(Setup {
            app,
            _folder: folder,
        })
    }

    /// A reply: its status line and its JSON body.
    struct Reply {
        status: String,
        body: Value,
    }

    fn call(app: &App, method: &str, target: &str, body: &str) -> Outcome<Reply> {
        let mut out = Vec::new();
        answer_in(&mut out, &Request::new(method, target, body), app)?;
        let text = String::from_utf8_lossy(&out).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        Ok(Reply {
            status: head.lines().next().unwrap_or_default().to_string(),
            body: serde_json::from_str(body).unwrap_or(Value::String(body.to_string())),
        })
    }

    #[test]
    fn routes_every_path() {
        let table = [
            ("GET", "/", Route::Page),
            ("GET", "/api/meta", Route::Meta),
            ("GET", "/api/orphans", Route::Orphans),
            ("GET", "/slop.jsonl", Route::SlopFile),
            ("GET", "/api/categories", Route::Categories),
            ("GET", "/api/page", Route::Records),
            ("GET", "/api/record", Route::Record),
            ("POST", "/api/flag", Route::SetFlag),
            ("GET", "/api/phrases", Route::Phrases),
            ("POST", "/api/flag-matches", Route::FlagMatches),
            ("GET", "/api/flag", Route::NotFound),
        ];

        for (method, path, route) in table {
            assert_eq!(Route::of(method, path), route, "{method} {path}");
        }
    }

    #[test]
    fn serves_the_removed_examples() -> Outcome {
        let setup = setup()?;
        let removed = call(&setup.app, "GET", "/slop.jsonl", "")?;
        assert_eq!(
            (removed.status.as_str(), &removed.body["id"]),
            ("HTTP/1.1 200 OK", &Value::from("gone"))
        );
        assert!(matches!(
            call(&setup.app, "DELETE", "/api/flag", ""),
            Err(ReviewError::NotFound(_))
        ));
        assert!(matches!(
            call(&setup.app, "GET", "/", ""),
            Err(ReviewError::NotFound(_))
        ));
        Ok(())
    }

    #[test]
    fn serves_the_page_the_dataset_list_and_each_dataset() -> Outcome {
        let folder = TempDir::new()?;
        let train = folder.file("train.jsonl", TRAINING)?;
        let teacher_line = r#"{"id":"t1","source":"teacher","origin":"trpl/src/ch08.md","entry":"grounded/trpl.md#1","based_on":"trpl/src/ch08.md","licence":"Apache-2.0","source_text":"Vectors hold values.","notes":["turn 2: R1"],"messages":[{"role":"user","content":"Q?"},{"role":"assistant","content":"A."}]}"#;
        let teacher = folder.file("teacher.jsonl", &format!("{teacher_line}\n"))?;
        let specs = [
            crate::workspace::DatasetSpec::new(
                "train",
                &train,
                &folder.path().join("f1.jsonl"),
                &folder.path().join("r1.jsonl"),
            ),
            crate::workspace::DatasetSpec::new(
                "teacher",
                &teacher,
                &folder.path().join("f2.jsonl"),
                &folder.path().join("r2.jsonl"),
            ),
        ];
        let workspace = Workspace::open(&specs)?;
        let mut page = Vec::new();
        answer(&mut page, &Request::new("GET", "/", ""), &workspace)?;
        assert!(String::from_utf8_lossy(&page).starts_with("HTTP/1.1 200 OK"));
        let mut listed = Vec::new();
        answer(
            &mut listed,
            &Request::new("GET", "/api/datasets", ""),
            &workspace,
        )?;
        assert!(String::from_utf8_lossy(&listed).contains(r#""name":"teacher","#));
        let mut records = Vec::new();
        answer(
            &mut records,
            &Request::new("GET", "/api/page?dataset=teacher", ""),
            &workspace,
        )?;
        let text = String::from_utf8_lossy(&records).into_owned();
        assert!(
            text.contains(r#""source_text":"Vectors hold values.""#)
                && text.contains(r#""licence":"Apache-2.0""#),
            "{text}"
        );
        assert!(text.contains(r#""collection_title":"The Rust Programming Language""#));
        Ok(())
    }

    #[test]
    fn meta_counts_sources_books_and_flags() -> Outcome {
        let setup = setup()?;
        let meta = call(&setup.app, "GET", "/api/meta", "")?.body;
        assert_eq!(
            (meta["total"].as_u64(), meta["flagged"].as_u64()),
            (Some(3), Some(4))
        );
        assert_eq!(meta["sources"].as_array().map(Vec::len), Some(3));
        assert_eq!(
            meta["collections"][0]["title"],
            "The Rust Programming Language"
        );
        Ok(())
    }

    #[test]
    fn orphans_are_reviewed_flags_with_no_example() -> Outcome {
        let setup = setup()?;
        let orphans = call(&setup.app, "GET", "/api/orphans", "")?.body;
        let expected = serde_json::json!({ "orphans": [
            { "id": "also-lost", "note": "rebuilt", "spans": [] },
            { "id": "lost", "note": "text changed", "spans": [] }
        ] });
        assert_eq!(orphans, expected);
        Ok(())
    }

    #[test]
    fn categories_carry_names_and_words() -> Outcome {
        let setup = setup()?;
        let body = call(&setup.app, "GET", "/api/categories", "")?.body;
        assert_eq!(
            body["categories"][0],
            serde_json::json!({ "name": "fake_importance", "description": "fake importance" })
        );
        assert_eq!(body["categories"].as_array().map(Vec::len), Some(8));
        Ok(())
    }

    #[test]
    fn pages_filter_and_suggest() -> Outcome {
        let setup = setup()?;
        let page = call(&setup.app, "GET", "/api/page?source=chat&limit=500", "")?.body;
        assert_eq!(page["total"], 1);
        let item = &page["items"][0];
        assert_eq!(
            (item["id"].as_str(), item["source_title"].as_str()),
            (Some("a1"), Some("Claude chat export"))
        );
        assert!(
            item["suggestions"]["slop"]
                .as_array()
                .is_some_and(|hits| !hits.is_empty())
        );
        let searched = call(&setup.app, "GET", "/api/page?q=here%27s+the+thing", "")?.body;
        assert_eq!(searched["total"], 2);
        Ok(())
    }

    #[test]
    fn a_record_comes_whole_and_unknown_positions_are_not_found() -> Outcome {
        let setup = setup()?;
        let record = call(&setup.app, "GET", "/api/record?position=2", "")?.body;
        assert_eq!(record["response"], "line one\nline two\nline three");
        assert_eq!(
            record["suggestions"],
            serde_json::json!({ "slop": [], "violations": [] })
        );
        assert!(matches!(
            call(&setup.app, "GET", "/api/record?position=99", ""),
            Err(ReviewError::NotFound(_))
        ));
        Ok(())
    }

    #[test]
    fn sets_and_clears_a_flag() -> Outcome {
        let setup = setup()?;
        let body = r#"{"id":"a1","note":"flattery","spans":[{"field":"response","text":"Great question!","category":"flattery_filler_opener"}]}"#;
        let set = call(&setup.app, "POST", "/api/flag", body)?.body;
        assert_eq!(set, serde_json::json!({ "ok": true, "flagged": true }));
        assert_eq!(
            setup.app.flags()?.get("a1").map(|flag| flag.spans.len()),
            Some(1)
        );
        call(
            &setup.app,
            "POST",
            "/api/flag",
            r#"{"id":"a1","flagged":false}"#,
        )?;
        assert_eq!(setup.app.flags()?.get("a1"), None);
        Ok(())
    }

    #[test]
    fn an_unknown_category_is_a_bad_request() -> Outcome {
        let setup = setup()?;
        let body = r#"{"id":"a1","spans":[{"field":"response","text":"x","category":"made_up"}]}"#;
        assert!(matches!(
            call(&setup.app, "POST", "/api/flag", body),
            Err(ReviewError::BadRequest(_))
        ));
        Ok(())
    }

    #[test]
    fn marks_a_phrase_everywhere_as_written() -> Outcome {
        let setup = setup()?;
        let body = r#"{"text":"here's the thing","category":"dramatic_setup"}"#;
        let first = call(&setup.app, "POST", "/api/flag-matches", body)?.body;
        let again = call(&setup.app, "POST", "/api/flag-matches", body)?.body;
        assert_eq!(first, serde_json::json!({ "matched": 2, "added": 2 }));
        assert_eq!(again, serde_json::json!({ "matched": 2, "added": 0 }));
        let marked = setup
            .app
            .flags()?
            .get("b2")
            .map(|flag| flag.spans[0].text.clone());
        assert_eq!(marked.as_deref(), Some("Here's The Thing"));
        let phrases = call(&setup.app, "GET", "/api/phrases", "")?.body;
        assert_eq!(phrases["phrases"][0]["examples"], 2);
        Ok(())
    }

    #[test]
    fn refuses_a_phrase_that_is_too_short() -> Outcome {
        let setup = setup()?;
        let outcome = call(
            &setup.app,
            "POST",
            "/api/flag-matches",
            r#"{"text":" ok ","category":"other"}"#,
        );
        assert!(outcome.is_err_and(|error| error.to_string().contains("at least 4 characters")));
        Ok(())
    }

    #[test]
    fn shortens_at_a_line_end_counting_characters() {
        assert_eq!(shorten_at_line("fn a() {}\nfn b() {}\n", 14), "fn a() {}");
        assert_eq!(shorten_at_line("abcdefgh", 3), "abc");
        assert_eq!(shorten_at_line("é\néé", 3), "é");
    }

    #[test]
    fn a_long_answer_is_sent_shortened_on_a_page() -> Outcome {
        let folder = TempDir::new()?;
        let long = "x".repeat(MAX_INLINE_RESPONSE + 10);
        let line = serde_json::json!({ "id": "l", "source": "chat", "origin": "c", "response": format!("short\n{long}") });
        let training = folder.file("train.jsonl", &format!("{line}\n"))?;
        let app = App::open(
            &training,
            &folder.path().join("f.jsonl"),
            &folder.path().join("s.jsonl"),
        )?;
        let item = &call(&app, "GET", "/api/page", "")?.body["items"][0];
        assert_eq!(
            (item["response"].as_str(), item["shortened"].as_bool()),
            (Some("short"), Some(true))
        );
        assert_eq!(
            item["response_chars"].as_u64(),
            u64::try_from(MAX_INLINE_RESPONSE + 16).ok()
        );
        Ok(())
    }
}
