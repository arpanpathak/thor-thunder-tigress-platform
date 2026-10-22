//! Which request goes where: the axum router, the check of the access key,
//! and one small handler per route.
//!
//! Everything that calls an engine is blocking code, so its handler hands the
//! work to [`bridge::answered`].

use std::{path::Path, sync::Arc};

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;

use crate::{
    server::{
        bridge,
        response::{self, Client, JSON},
    },
    chat,
    config::{BEARER, Config},
    error::Outcome,
    engines::{messages, models},
    paths,
    web::search,
};

/// What `/health` answers.
const HEALTHY: &str = r#"{"status":"ok"}"#;

/// What the server sends back when the access key is missing or wrong, in
/// the error format OpenAI clients expect.
const INVALID_KEY: &str =
    r#"{"error":{"code":401,"message":"Invalid API Key","type":"authentication_error"}}"#;

/// The largest request body accepted: a long conversation with pasted code.
const MAX_BODY: usize = 32 << 20;

/// The header Anthropic clients send the key in.
const API_KEY: &str = "x-api-key";

/// A file the server hands out, with its content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StaticFile {
    name: &'static str,
    content_type: &'static str,
}

/// The chat page.
const PAGE: StaticFile = StaticFile {
    name: "index.html",
    content_type: "text/html; charset=utf-8",
};

/// The other files in the web folder that may be served; nothing else is.
const FILES: [StaticFile; 14] = [
    StaticFile {
        name: "about.html",
        content_type: "text/html; charset=utf-8",
    },
    StaticFile {
        name: "chat.css",
        content_type: "text/css; charset=utf-8",
    },
    StaticFile {
        name: "chat.js",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "appearance.js",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "dom.js",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "markdown.js",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "history.js",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "stream.js",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "rich.js",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "attachments.js",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "themes.css",
        content_type: "text/css; charset=utf-8",
    },
    StaticFile {
        name: "messages.css",
        content_type: "text/css; charset=utf-8",
    },
    StaticFile {
        name: "cub.svg",
        content_type: "image/svg+xml",
    },
    StaticFile {
        name: "cub.png",
        content_type: "image/png",
    },
];

/// Libraries the page loads the first time a reply or a file needs one: KaTeX
/// for formulas, Mermaid for diagrams, PDF.js for attached PDFs. Each name
/// carries its version, so a browser may keep it for good: a new version is a
/// new name.
const LIBRARIES: [StaticFile; 5] = [
    StaticFile {
        name: "katex-0.19.0.min.js",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "katex-0.19.0.css",
        content_type: "text/css; charset=utf-8",
    },
    StaticFile {
        name: "mermaid-12.1.0.min.js",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "pdf-6.4.299.min.mjs",
        content_type: "text/javascript; charset=utf-8",
    },
    StaticFile {
        name: "pdf.worker-6.4.299.min.mjs",
        content_type: "text/javascript; charset=utf-8",
    },
];

/// How long a browser keeps a library: a year, without asking again.
const KEPT: &str = "public, max-age=31536000, immutable";

/// The settings, shared by every request.
type Shared = State<Arc<Config>>;

/// Every route of the server. A path that is no route is looked up among the
/// listed files; a route asked with the wrong method is not found.
pub fn router(config: Arc<Config>) -> Router {
    Router::new()
        .route(paths::HEALTH, get(health))
        .route(paths::MODELS, get(models))
        .route(paths::CHAT_COMPLETIONS, post(chat_completions))
        .route(paths::MESSAGES, post(messages))
        .route(paths::COUNT_TOKENS, post(count_tokens))
        .route(paths::SEARCH, get(web_search))
        .route(paths::REQUEST, post(request_access))
        .fallback(file)
        .method_not_allowed_fallback(not_found)
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn_with_state(Arc::clone(&config), guard))
        .with_state(config)
}

/// Answers a CORS preflight itself, refuses a request that touches the model
/// (`/v1/…`) without the key, and passes every other request on. Every
/// answer may be read by pages on other sites, and none is kept by a cache
/// unless its handler says otherwise (the versioned libraries).
///
/// Letting other sites call is safe because access is by a key in a header,
/// not by cookies: a site can't use a visitor's key without having it.
async fn guard(State(config): Shared, request: Request, next: Next) -> Response {
    let needs_key = request.uri().path().starts_with(paths::API);
    let key = sent_key(request.headers());

    let mut response = if request.method() == Method::OPTIONS {
        preflight()
    } else if needs_key && !config.admits(key.as_deref()) {
        unauthorized()
    } else {
        next.run(request).await
    };

    let headers = response.headers_mut();

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));

    response
}

/// The `Authorization` value a request carries; an `x-api-key` header
/// (Anthropic clients) counts as `Bearer <key>`.
fn sent_key(headers: &HeaderMap) -> Option<String> {
    let text = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());

    text(header::AUTHORIZATION.as_str())
        .map(str::to_string)
        .or_else(|| text(API_KEY).map(|key| format!("{BEARER}{key}")))
}

/// The answer to a CORS preflight: browsers ask before a cross-site request
/// that carries an `Authorization` header.
fn preflight() -> Response {
    (
        StatusCode::NO_CONTENT,
        [
            (header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS"),
            (
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                "Authorization, Content-Type, x-api-key, anthropic-version",
            ),
            (header::ACCESS_CONTROL_MAX_AGE, "86400"),
        ],
    )
        .into_response()
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, JSON)],
        INVALID_KEY,
    )
        .into_response()
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

async fn health() -> Response {
    ([(header::CONTENT_TYPE, JSON)], HEALTHY).into_response()
}

async fn models(State(config): Shared) -> Response {
    bridge::answered(move |client| models::list(client, &config.upstreams)).await
}

async fn chat_completions(State(config): Shared, body: Bytes) -> Response {
    bridge::answered(move |client| chat::answer(client, &body, &config.upstreams)).await
}

async fn messages(State(config): Shared, body: Bytes) -> Response {
    let forward = move |client: &mut dyn Client| {
        messages::forward(client, paths::MESSAGES, &body, &config.upstreams)
    };

    bridge::answered(forward).await
}

async fn count_tokens(State(config): Shared, body: Bytes) -> Response {
    let forward = move |client: &mut dyn Client| {
        messages::forward(client, paths::COUNT_TOKENS, &body, &config.upstreams)
    };

    bridge::answered(forward).await
}

/// The query of a `/v1/search` request.
#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

/// Searches the local SearXNG for a remote coding tool and answers in
/// SearXNG's own JSON shape, so a client written for SearXNG reads it as is.
async fn web_search(State(config): Shared, Query(query): Query<SearchQuery>) -> Response {
    let answer = move |client: &mut dyn Client| {
        let hits = search::search_all(&config.upstreams.search, &[query.q], None)
            .into_iter()
            .next()
            .unwrap_or_else(|| Ok(search::Hits::default()))?;

        let results: Vec<_> = hits
            .results
            .iter()
            .map(|hit| {
                serde_json::json!({
                    "title": hit.title,
                    "url": hit.url,
                    "content": hit.snippet,
                    "publishedDate": hit.published,
                })
            })
            .collect();
        let body = serde_json::json!({ "results": results, "unresponsive_engines": hits.down });

        response::respond(client, StatusCode::OK, JSON, body.to_string().as_bytes())
    };

    bridge::answered(answer).await
}

/// Records a registration form's name and email in the keyring. It is open,
/// because it is how someone without a key asks for one; an email that is
/// already waiting is not added twice.
async fn request_access(State(config): Shared, body: Bytes) -> Response {
    #[derive(Deserialize)]
    struct Form {
        name: String,
        email: String,
    }

    let record = move |client: &mut dyn Client| {
        let form: Form = serde_json::from_slice(&body)?;

        config.request_access(&form.name, &form.email)?;

        response::respond(client, StatusCode::OK, JSON, br#"{"status":"recorded"}"#)
    };

    bridge::answered(record).await
}

/// The path inside the site, with the `/thor-tigress-cub` folder removed.
fn within_site(path: &str) -> &str {
    path.strip_prefix(paths::CUB).unwrap_or(path)
}

/// Whether `path` is the chat page: `/`, `/index.html`, or the same under
/// `/thor-tigress-cub`.
fn is_page(path: &str) -> bool {
    matches!(within_site(path), "" | "/" | "/index.html")
}

/// The file a path names: the chat page, or a listed file at the root or
/// under `/thor-tigress-cub/`.
fn listed_file(path: &str) -> Option<StaticFile> {
    if is_page(path) {
        return Some(PAGE);
    }

    let name = within_site(path).strip_prefix('/')?;

    FILES
        .into_iter()
        .chain(LIBRARIES)
        .find(|file| file.name == name)
}

/// Answers a `GET` for anything that is no route: a listed file, or not found.
async fn file(State(config): Shared, method: Method, uri: Uri) -> Outcome<Response> {
    let listed = (method == Method::GET)
        .then(|| listed_file(uri.path()))
        .flatten();

    match listed {
        Some(file) => send_file(&config.web, file).await,
        None => Ok(not_found().await),
    }
}

async fn send_file(folder: &Path, file: StaticFile) -> Outcome<Response> {
    let body = tokio::fs::read(folder.join(file.name)).await?;

    let mut response = ([(header::CONTENT_TYPE, file.content_type)], body).into_response();

    if LIBRARIES.contains(&file) {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(KEPT));
    }

    Ok(response)
}

#[cfg(test)]
mod tests;
