//! Shows the training set to a person and records slop flags.
//!
//! ```text
//!   cargo run --release -- [TRAIN_JSONL] [PORT] [FLAGS_JSONL] [SLOP_JSONL]
//!   cargo run --release -- scan  [TRAIN_JSONL]  [AUTO_FLAGS_JSONL]
//!   cargo run --release -- apply [AUTO_FLAGS_JSONL] [FLAGS_JSONL]
//! ```
//!
//! `apply` turns the scan's suggestions into real flags, so a person can then
//! review them and clear the false positives. It never overwrites a flag a
//! person set.
//!
//! The whole corpus is never held in memory: [`index`] keeps only each record's
//! byte offset, and a page is answered by reading the bytes it shows.

mod error;
mod flags;
mod http;
mod index;
mod scan;
mod titles;

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{Arc, Mutex},
};

use serde::Deserialize;
use serde_json::{json, Value};

use thor_spark_safety_eval::{answer, slop::Category};

use crate::{
    error::ReviewError,
    flags::{Flag, FlagStore, Span, CATEGORIES},
    http::{read_request, write_json, write_response, Request},
    index::{Filter, Index},
};

/// The page, compiled into the binary.
const PAGE: &str = include_str!("page.html");

/// The most records one page may ask for.
const MAX_PAGE: usize = 100;

/// The default number of records per page.
const DEFAULT_PAGE: usize = 20;

/// The longest answer sent inline with a page, in characters. A longer answer is
/// sent shortened and fetched in full only when the reader asks for it, so one
/// enormous record cannot make a whole page heavy. The longest answer in the
/// corpus is over 380,000 characters. At 6,000 most single code files arrive
/// whole, so a reviewer can read the code without expanding it.
const MAX_INLINE_RESPONSE: usize = 6_000;

/// The sources written by the reviewer. They are the reference, and they show
/// bad code on purpose to contrast it with good code, so the page suggests
/// nothing in them.
const OWN_SOURCES: [&str; 2] = ["readability", "clever_vs_readable"];

/// The training file the tool reads when no argument is given.
const DEFAULT_TRAINING: &str = "data/train.jsonl";

/// The flags file the tool writes when no argument is given.
const DEFAULT_FLAGS: &str = "labels/slop_flags.jsonl";

/// The examples already removed as slop, shown alongside the training set.
const DEFAULT_SLOP: &str = "data/slop.jsonl";

/// Where the scan writes the flags it suggests.
const DEFAULT_AUTO_FLAGS: &str = "labels/auto_flags.jsonl";

/// What a request to change a flag carries.
#[derive(Deserialize)]
struct FlagRequest {
    /// The example id.
    id: String,
    /// The reviewer's note.
    #[serde(default)]
    note: String,
    /// The marked phrases.
    #[serde(default)]
    spans: Vec<Span>,
    /// False clears the flag.
    #[serde(default = "flag_default")]
    flagged: bool,
}

/// The default for [`FlagRequest::flagged`], so a body that omits it sets a flag.
fn flag_default() -> bool {
    true
}

/// The index and the flags, shared by every connection.
struct App {
    /// The training file.
    index: Index,
    /// The reviewer's flags.
    flags: Mutex<FlagStore>,
    /// The examples already removed as slop by a previous build.
    slop: PathBuf,
    /// The ids of those removed examples, so a flag that points at one is not
    /// mistaken for an orphan.
    slop_ids: HashSet<String>,
}

impl App {
    /// The flagged ids, or an empty set when the lock is poisoned.
    fn flagged_ids(&self) -> std::collections::HashSet<String> {
        match self.flags.lock() {
            Ok(store) => store.ids(),
            Err(_) => std::collections::HashSet::new(),
        }
    }

    /// How many examples are flagged.
    fn flagged_count(&self) -> usize {
        match self.flags.lock() {
            Ok(store) => store.len(),
            Err(_) => 0,
        }
    }
}

/// Builds the index and serves until interrupted.
fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let first = arguments.next();
    if first.as_deref() == Some("scan") {
        let training = PathBuf::from(arguments.next().unwrap_or_else(|| DEFAULT_TRAINING.to_string()));
        let out = PathBuf::from(arguments.next().unwrap_or_else(|| DEFAULT_AUTO_FLAGS.to_string()));
        return match suggest_flags(&training, &out) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("reinforcer: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if first.as_deref() == Some("apply") {
        let suggestions = PathBuf::from(arguments.next().unwrap_or_else(|| DEFAULT_AUTO_FLAGS.to_string()));
        let flags_path = PathBuf::from(arguments.next().unwrap_or_else(|| DEFAULT_FLAGS.to_string()));
        return match apply_suggestions(&suggestions, &flags_path) {
            Ok(added) => {
                println!(
                    "applied {added} suggestions to {}",
                    flags_path.display()
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("reinforcer: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let training = PathBuf::from(first.unwrap_or_else(|| DEFAULT_TRAINING.to_string()));
    let port = arguments.next().unwrap_or_else(|| "8080".to_string());
    let flags_path = PathBuf::from(arguments.next().unwrap_or_else(|| DEFAULT_FLAGS.to_string()));
    let slop_path = PathBuf::from(arguments.next().unwrap_or_else(|| DEFAULT_SLOP.to_string()));
    match run(&training, &port, &flags_path, &slop_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("reinforcer: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Reads the corpus, writes the suggested flags, and prints what was found.
///
/// Nothing is removed and no human flag is touched: this only proposes.
fn suggest_flags(training: &Path, out: &Path) -> Result<(), ReviewError> {
    let index = Index::open(training)?;
    let suggestions = scan::scan(&index)?;
    scan::write(out, &suggestions)?;
    println!("scanned {} records", index.len());
    for (rule, count) in scan::counts(&suggestions) {
        println!("  {:<24} {:>6}", rule, count);
    }
    println!(
        "  {:<24} {:>6} distinct examples",
        "total",
        scan::examples(&suggestions)
    );
    println!("wrote {}", out.display());
    Ok(())
}

/// Turns every suggestion into a flag, unless a person already flagged that
/// example. A person's note is never overwritten. Returns how many flags were
/// added.
fn apply_suggestions(suggestions_path: &Path, flags_path: &Path) -> Result<usize, ReviewError> {
    let suggestions = scan::read_suggestions(suggestions_path)?;
    let mut by_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for suggestion in &suggestions {
        by_id.entry(suggestion.id.clone()).or_default().push(format!(
            "{} [{}] in {} matched {:?} in {}",
            suggestion.rule,
            suggestion.category,
            suggestion.source,
            suggestion.matched,
            suggestion.field
        ));
    }
    let mut store = FlagStore::open(flags_path)?;
    let mut added = 0usize;
    for (id, reasons) in by_id {
        if store.get(&id).is_some() {
            continue;
        }
        store.set(
            &id,
            Flag {
                note: format!("auto: {}", reasons.join("; ")),
                spans: Vec::new(),
            },
        );
        added += 1;
    }
    store.save()?;
    Ok(added)
}

/// Opens the files, binds the listener and serves.
fn run(training: &Path, port: &str, flags_path: &Path, slop_path: &Path) -> Result<(), ReviewError> {
    let index = Index::open(training)?;
    let flags = FlagStore::open(flags_path)?;
    let removed_ids = read_slop_ids(slop_path)?;
    println!("indexed {} records from {}", index.len(), training.display());
    println!("flags are saved to {}", flags_path.display());
    let address = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&address).map_err(|error| {
        ReviewError::BadRequest(format!("binding {address}: {error}"))
    })?;
    println!("open http://{address}");
    let app = Arc::new(App {
        index,
        flags: Mutex::new(flags),
        slop: slop_path.to_path_buf(),
        slop_ids: removed_ids,
    });
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let app = Arc::clone(&app);
                std::thread::spawn(move || handle(stream, &app));
            }
            Err(error) => eprintln!("reinforcer: {error}"),
        }
    }
    Ok(())
}

/// Answers one connection, reporting a failure to the client.
fn handle(mut stream: TcpStream, app: &App) {
    if let Err(error) = respond(&mut stream, app) {
        eprintln!("reinforcer: {error}");
        let status = error.status();
        let body = json!({ "error": error.to_string() });
        write_json(&mut stream, status, &body).ok();
    }
}

/// Routes one request.
fn respond(stream: &mut TcpStream, app: &App) -> Result<(), ReviewError> {
    let request = read_request(stream)?;
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => write_response(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
        ),
        ("GET", "/api/meta") => write_json(stream, "200 OK", &meta(app)),
        ("GET", "/api/orphans") => write_json(stream, "200 OK", &orphans(app)),
        ("GET", "/slop.jsonl") => write_response(
            stream,
            "200 OK",
            "application/x-ndjson; charset=utf-8",
            &fs::read(&app.slop).unwrap_or_default(),
        ),
        ("GET", "/api/categories") => write_json(stream, "200 OK", &categories()),
        ("GET", "/api/page") => write_json(stream, "200 OK", &page(app, &request)?),
        ("GET", "/api/record") => write_json(stream, "200 OK", &record(app, &request)?),
        ("POST", "/api/flag") => write_json(stream, "200 OK", &set_flag(app, &request)?),
        ("GET", "/api/phrases") => write_json(stream, "200 OK", &phrases(app)),
        ("POST", "/api/flag-matches") => write_json(stream, "200 OK", &flag_matches(app, &request)?),
        (method, path) => Err(ReviewError::NotFound(format!("{method} {path}"))),
    }
}

/// The counts the header shows.
fn meta(app: &App) -> Value {
    let sources: Vec<Value> = app
        .index
        .counts()
        .into_iter()
        .map(|(name, count)| json!({ "name": name, "title": titles::source_title(&name), "count": count }))
        .collect();
    let collections: Vec<Value> = app
        .index
        .collections()
        .into_iter()
        .map(|(folder, source, count)| {
            json!({ "name": folder, "title": titles::collection_title(&folder), "source": source, "count": count })
        })
        .collect();
    json!({
        "total": app.index.len(),
        "flagged": app.flagged_count(),
        "sources": sources,
        "collections": collections,
        "file": app.index.path().display().to_string(),
    })
}

/// The flags whose example is in neither the training set nor the slop file.
///
/// A rebuild that changed an example's text leaves its flag here: the id is a
/// hash of the text, so it can no longer find its example. The page shows these
/// so a person can re-point them instead of losing them.
fn orphans(app: &App) -> Value {
    let entries = match app.flags.lock() {
        Ok(store) => store.all(),
        Err(_) => Vec::new(),
    };
    let items: Vec<Value> = entries
        .into_iter()
        .filter(|(id, flag)| !app.index.contains(id) && !app.slop_ids.contains(id) && flag.is_reviewed())
        .map(|(id, flag)| json!({ "id": id, "note": flag.note, "spans": flag.spans }))
        .collect();
    json!({ "orphans": items })
}

/// The ids of the examples a previous build removed, for orphan detection.
///
/// A missing file means none: the tool still runs before the first build.
fn read_slop_ids(path: &Path) -> Result<HashSet<String>, ReviewError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(error) => return Err(ReviewError::io(path)(error)),
    };
    let mut ids = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(ReviewError::json(path, index + 1))?;
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            ids.insert(id.to_string());
        }
    }
    Ok(ids)
}

/// The categories the picker offers.
fn categories() -> Value {
    let items: Vec<Value> = CATEGORIES
        .iter()
        .map(|(name, description)| json!({ "name": name, "description": description }))
        .collect();
    json!({ "categories": items })
}

/// One page of records.
fn page(app: &App, request: &Request) -> Result<Value, ReviewError> {
    let filter = Filter {
        source: non_empty(request.param("source")),
        query: non_empty(request.param("q")),
        flagged: request.boolean("flagged"),
        collection: non_empty(request.param("collection")),
    };
    let matches = app.index.matching(&filter, &app.flagged_ids())?;
    let total = matches.len();
    let start = request.number("start", 0).min(total);
    let limit = request.number("limit", DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let mut items = Vec::new();
    for position in matches.iter().skip(start).take(limit) {
        items.push(item(app, *position, Some(MAX_INLINE_RESPONSE))?);
    }
    Ok(json!({ "start": start, "total": total, "items": items }))
}

/// One whole record, for an answer a page sent shortened.
fn record(app: &App, request: &Request) -> Result<Value, ReviewError> {
    item(app, request.number("position", usize::MAX), None)
}

/// One record, with its flag if it has one.
///
/// `inline_limit` shortens the answer so a page stays small. The length of the
/// whole answer is always sent, so the reader can be told what is held back.
fn item(app: &App, position: usize, inline_limit: Option<usize>) -> Result<Value, ReviewError> {
    let record = app.index.record(position)?;
    let parsed: Value = serde_json::from_str(&record)
        .map_err(|error| ReviewError::BadRequest(format!("record {position}: {error}")))?;
    let entry = app
        .index
        .entry(position)
        .ok_or_else(|| ReviewError::NotFound(format!("record {position}")))?;
    let flag = match app.flags.lock() {
        Ok(store) => store.get(&entry.id).cloned(),
        Err(_) => None,
    };
    let response = field(&parsed, "response");
    let response_chars = response.chars().count();
    let shortened = inline_limit.is_some_and(|limit| response_chars > limit);
    let inline = match (shortened, inline_limit) {
        (true, Some(limit)) => shorten_at_line(&response, limit),
        _ => response,
    };
    Ok(json!({
        "position": position,
        "id": entry.id,
        "source": app.index.source_of(entry),
        "source_title": titles::source_title(app.index.source_of(entry)),
        "collection_title": app.index.collection_of(entry).map(titles::collection_title),
        "origin": app.index.origin_of(entry),
        "instruction": field(&parsed, "instruction"),
        "response": inline,
        "response_chars": response_chars,
        "shortened": shortened,
        "flag": flag,
        "messages": parsed.get("messages").cloned().unwrap_or(Value::Null),
        "rejected": parsed.get("rejected").cloned().unwrap_or(Value::Null),
        "suggestions": match OWN_SOURCES.contains(&app.index.source_of(entry)) {
            true => json!({ "slop": [], "violations": [] }),
            false => suggestions(&field(&parsed, "response")),
        },
    }))
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

/// What Stage 0 finds in one answer, in the shape the page draws: slop
/// phrases to suggest, and code lines that break one of the five rules. A code
/// line is sent as its text, so the page can find it in whatever block it
/// renders without both sides having to agree on how blocks are counted.
fn suggestions(response: &str) -> Value {
    let score = answer::score(response);
    let lines: Vec<&str> = response.lines().collect();
    let slop: Vec<Value> = score
        .slop
        .hits
        .iter()
        .map(|hit| json!({ "text": hit.text, "category": category_name(hit.category) }))
        .collect();
    let violations: Vec<Value> = score
        .blocks
        .iter()
        .flat_map(|block| {
            block.report.violations.iter().map(|violation| {
                let line = lines
                    .get((block.line + violation.line).saturating_sub(2))
                    .map(|text| text.trim())
                    .unwrap_or_default();
                json!({ "rule": titles::rule_title(violation.rule), "detail": violation.detail, "line": line })
            })
        })
        .collect();
    json!({ "slop": slop, "violations": violations })
}

/// The flag category name of a Stage 0 slop category.
fn category_name(category: Category) -> &'static str {
    match category {
        Category::FakeImportance => "fake_importance",
        Category::DramaticSetup => "dramatic_setup",
        Category::EmptyDepthWords => "empty_depth_words",
        Category::FakeBalanceHedging => "fake_balance_hedging",
        Category::FlatteryFillerOpener => "flattery_filler_opener",
        Category::WrapUpRepeat => "wrap_up_repeat",
        Category::RhythmTrick => "rhythm_trick",
    }
}

/// Every phrase a reviewer has marked, for the page to highlight everywhere.
fn phrases(app: &App) -> Value {
    let phrases = match app.flags.lock() {
        Ok(store) => store.phrases(),
        Err(_) => Vec::new(),
    };
    json!({ "phrases": phrases })
}

/// What a request to flag every example containing a phrase carries.
#[derive(Deserialize)]
struct MatchRequest {
    /// The phrase, matched without regard to case.
    text: String,
    /// The slop category to mark it as.
    category: String,
}

/// Marks `text` in the response of every example that contains it. Examples
/// that already carry the phrase are left as they are.
fn flag_matches(app: &App, request: &Request) -> Result<Value, ReviewError> {
    let parsed: MatchRequest = serde_json::from_str(&request.body)
        .map_err(|error| ReviewError::BadRequest(format!("flag-matches body: {error}")))?;
    let needle = parsed.text.trim().to_lowercase();
    if needle.chars().count() < 4 || !FlagStore::is_known_category(&parsed.category) {
        return Err(ReviewError::BadRequest("phrase too short or unknown category".to_string()));
    }
    let filter = Filter {
        query: Some(needle.clone()),
        ..Filter::default()
    };
    let positions = app.index.matching(&filter, &HashSet::new())?;
    let mut found = Vec::new();
    for position in positions {
        let record: Value = serde_json::from_str(&app.index.record(position)?)
            .map_err(|error| ReviewError::BadRequest(format!("record {position}: {error}")))?;
        let response = field(&record, "response");
        let id = field(&record, "id");
        if let Some(start) = response.to_lowercase().find(&needle)
            && let Some(text) = response.get(start..start + needle.len())
        {
            found.push((id, text.to_string()));
        }
    }
    let mut store = app
        .flags
        .lock()
        .map_err(|_| ReviewError::BadRequest("flag store is poisoned".to_string()))?;
    let note = format!("phrase: {}", parsed.text.trim());
    let added = found
        .iter()
        .filter(|(id, text)| {
            let span = Span {
                field: "response".to_string(),
                text: text.clone(),
                category: parsed.category.clone(),
            };
            store.add_span(id, span, &note)
        })
        .count();
    store.save()?;
    Ok(json!({ "matched": found.len(), "added": added }))
}

/// A string field of a record, or an empty string when it is missing.
fn field(record: &Value, name: &str) -> String {
    record
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A parameter that counts only when it has text.
fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
}

/// Sets or clears one flag, and writes the file.
fn set_flag(app: &App, request: &Request) -> Result<Value, ReviewError> {
    let parsed: FlagRequest = serde_json::from_str(&request.body)
        .map_err(|error| ReviewError::BadRequest(format!("flag body: {error}")))?;
    for span in &parsed.spans {
        if !FlagStore::is_known_category(&span.category) {
            return Err(ReviewError::BadRequest(format!(
                "unknown category {}",
                span.category
            )));
        }
    }
    let mut store = app
        .flags
        .lock()
        .map_err(|_| ReviewError::BadRequest("flag store is poisoned".to_string()))?;
    match parsed.flagged {
        true => store.set(
            &parsed.id,
            Flag {
                note: parsed.note,
                spans: parsed.spans,
            },
        ),
        false => store.clear(&parsed.id),
    }
    store.save()?;
    Ok(json!({ "ok": true, "flagged": parsed.flagged }))
}

#[cfg(test)]
mod tests {
    use super::shorten_at_line;

    #[test]
    fn keeps_whole_lines_within_the_limit() {
        assert_eq!(shorten_at_line("fn a() {}\nfn b() {}\n", 14), "fn a() {}");
    }

    #[test]
    fn cuts_a_single_long_line_at_the_limit() {
        assert_eq!(shorten_at_line("abcdefgh", 3), "abc");
    }

    #[test]
    fn counts_characters_not_bytes() {
        assert_eq!(shorten_at_line("é\néé", 3), "é");
    }
}
