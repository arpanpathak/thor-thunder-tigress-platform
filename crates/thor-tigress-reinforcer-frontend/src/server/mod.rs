//! The review page and its API, served with axum.
//!
//! Every route except the page, the dataset list and the categories works on
//! the dataset named by `?dataset=`, the first one when not named. A handler
//! here only reads the request and picks the dataset; the answers are built
//! in [`overview`], [`mod@records`] and [`flagging`].

mod flagging;
mod overview;
mod params;
mod records;
#[cfg(test)]
mod tests;

use std::{path::Path, sync::Arc};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderValue, Method, Uri, header},
    middleware,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use tokio::net::TcpListener;

use self::{
    flagging::{FlagSaved, Matched, Orphans, Phrases},
    overview::{Categories, Datasets, Meta},
    params::Params,
    records::{Item, Length, Page},
};
use crate::{
    error::{Outcome, ReviewError},
    datasets::workspace::Workspace,
};

/// The page, compiled into the binary.
const PAGE: &str = include_str!("../page.html");

/// The largest request body accepted, in bytes.
const MAX_BODY: usize = 1 << 20;

/// The content type of the removed examples: one JSON object per line.
const JSON_LINES: &str = "application/x-ndjson; charset=utf-8";

/// The datasets, shared by every request.
type Shared = State<Arc<Workspace>>;

/// A JSON answer, or the error that becomes its status.
type Answer<T> = Outcome<Json<T>>;

/// Every route of the tool. An unknown path, or a known path asked with the
/// wrong method, is answered 404.
pub fn router(workspace: Arc<Workspace>) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/api/datasets", get(datasets))
        .route("/api/meta", get(meta))
        .route("/api/orphans", get(orphans))
        .route("/slop.jsonl", get(removed))
        .route("/api/categories", get(categories))
        .route("/api/page", get(records))
        .route("/api/record", get(record))
        .route("/api/flag", post(set_flag))
        .route("/api/phrases", get(phrases))
        .route("/api/flag-matches", post(flag_matches))
        .fallback(not_found)
        .method_not_allowed_fallback(not_found)
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::map_response(no_store))
        .with_state(workspace)
}

/// Answers requests on `listener` until `stop` completes; requests already
/// being answered are finished first.
///
/// # Errors
///
/// `ReviewError::Io` when the listener fails.
pub async fn serve(
    listener: TcpListener,
    workspace: Arc<Workspace>,
    stop: impl Future<Output = ()> + Send + 'static,
) -> Outcome {
    axum::serve(listener, router(workspace))
        .with_graceful_shutdown(stop)
        .await
        .map_err(ReviewError::io(Path::new("server")))
}

impl IntoResponse for ReviewError {
    fn into_response(self) -> Response {
        eprintln!("reinforcer: {self}");

        let body = json!({ "error": self.to_string() });

        (self.status(), Json(body)).into_response()
    }
}

/// Tells the browser to keep no answer: every one can change with the next flag.
async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));

    response
}

async fn not_found(method: Method, uri: Uri) -> ReviewError {
    ReviewError::NotFound(format!("{method} {}", uri.path()))
}

async fn page() -> Html<&'static str> {
    Html(PAGE)
}

async fn datasets(State(workspace): Shared) -> Answer<Datasets> {
    Ok(Json(overview::datasets(&workspace)?))
}

async fn meta(State(workspace): Shared, Query(params): Query<Params>) -> Answer<Meta> {
    Ok(Json(overview::meta(params.app(&workspace)?)?))
}

async fn categories() -> Json<Categories> {
    Json(overview::categories())
}

async fn orphans(State(workspace): Shared, Query(params): Query<Params>) -> Answer<Orphans> {
    Ok(Json(flagging::orphans(params.app(&workspace)?)?))
}

async fn removed(State(workspace): Shared, Query(params): Query<Params>) -> Outcome<Response> {
    let lines = records::removed(params.app(&workspace)?);

    Ok(([(header::CONTENT_TYPE, JSON_LINES)], lines).into_response())
}

async fn records(State(workspace): Shared, Query(params): Query<Params>) -> Answer<Page> {
    Ok(Json(records::records(params.app(&workspace)?, &params)?))
}

async fn record(State(workspace): Shared, Query(params): Query<Params>) -> Answer<Item> {
    let position = params.number("position", usize::MAX);

    Ok(Json(records::item(
        params.app(&workspace)?,
        position,
        Length::Whole,
    )?))
}

async fn set_flag(
    State(workspace): Shared,
    Query(params): Query<Params>,
    body: String,
) -> Answer<FlagSaved> {
    Ok(Json(flagging::set_flag(params.app(&workspace)?, &body)?))
}

async fn phrases(State(workspace): Shared, Query(params): Query<Params>) -> Answer<Phrases> {
    let phrases = params.app(&workspace)?.flags()?.phrases();

    Ok(Json(Phrases { phrases }))
}

async fn flag_matches(
    State(workspace): Shared,
    Query(params): Query<Params>,
    body: String,
) -> Answer<Matched> {
    Ok(Json(flagging::flag_matches(
        params.app(&workspace)?,
        &body,
    )?))
}
