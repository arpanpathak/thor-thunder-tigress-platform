use std::{fs, path::Path};

use anyhow::Result;
use axum::body::{Body, to_bytes};
use thor_tigress_keyring::store::Keyring;
use tower::ServiceExt;

use super::*;
use crate::{
    config::Engine,
    testing::{FakeServer, json_response},
};

const QUESTION: &str = r#"{"model":"qwen","messages":[{"role":"user","content":"hi"}]}"#;

/// An answer: its status, the headers the tests read, and its body.
struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl Reply {
    fn header(&self, name: header::HeaderName) -> &str {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
    }
}

/// A request to send: its method, path, headers and body.
struct Asked<'a> {
    method: &'a str,
    path: &'a str,
    headers: &'a [(&'a str, &'a str)],
    body: &'a str,
}

fn get(path: &str) -> Asked<'_> {
    Asked {
        method: "GET",
        path,
        headers: &[],
        body: "",
    }
}

fn post<'a>(path: &'a str, body: &'a str) -> Asked<'a> {
    Asked {
        method: "POST",
        path,
        headers: &[],
        body,
    }
}

impl<'a> Asked<'a> {
    fn with(self, headers: &'a [(&'a str, &'a str)]) -> Asked<'a> {
        Asked { headers, ..self }
    }
}

/// A web folder holding the page and its style sheet, removed when dropped.
fn web_folder() -> Result<tempfile::TempDir> {
    let folder = tempfile::tempdir()?;

    fs::write(folder.path().join("index.html"), "<p>cub</p>")?;
    fs::write(folder.path().join("chat.css"), "p {}")?;

    Ok(folder)
}

/// A server that serves `web`, with one fake engine and maybe a key.
fn setup(web: &Path, responses: Vec<String>, key: Option<&str>) -> Result<Arc<Config>> {
    let model = FakeServer::start(responses)?;
    let arguments = [
        "--web".to_string(),
        web.to_string_lossy().into_owned(),
        "--engine".to_string(),
        format!("nemotron={}", model.address()),
        "--key-file".to_string(),
        "/nonexistent".to_string(),
    ];
    let mut config = Config::from_args(arguments)?;

    config.key = key.map(ToString::to_string);

    Ok(Arc::new(config))
}

async fn ask(config: &Arc<Config>, asked: Asked<'_>) -> Result<Reply> {
    let builder = asked.headers.iter().fold(
        Request::builder().method(asked.method).uri(asked.path),
        |builder, (name, value)| builder.header(*name, *value),
    );
    let request = builder.body(Body::from(asked.body.to_string()))?;
    let Ok(response) = router(Arc::clone(config)).oneshot(request).await;
    let (status, headers) = (response.status(), response.headers().clone());
    let body = to_bytes(response.into_body(), usize::MAX).await?;

    Ok(Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

#[test]
fn serves_the_page_under_each_of_its_addresses() {
    for path in [
        "/",
        "/thor-tigress-cub/",
        "/thor-tigress-cub",
        "/index.html",
    ] {
        assert_eq!(listed_file(path), Some(PAGE), "{path}");
    }
}

#[test]
fn serves_only_the_listed_files() {
    let listed = [
        ("/about.html", "about.html"),
        ("/chat.css", "chat.css"),
        ("/thor-tigress-cub/chat.js", "chat.js"),
        ("/thor-tigress-cub/cub.svg", "cub.svg"),
        ("/cub.png", "cub.png"),
        ("/dom.js", "dom.js"),
        ("/appearance.js", "appearance.js"),
        ("/thor-tigress-cub/markdown.js", "markdown.js"),
        ("/history.js", "history.js"),
        ("/themes.css", "themes.css"),
        ("/thor-tigress-cub/messages.css", "messages.css"),
        ("/rich.js", "rich.js"),
        ("/thor-tigress-cub/attachments.js", "attachments.js"),
    ];
    let unlisted = [
        "/serve.sh",
        "/../../.config/thor-chat/api-key",
        "/thor-tigress-cub/../serve.sh",
        "about.html",
    ];

    for (path, name) in listed {
        let found = listed_file(path).map(|file| file.name);

        assert_eq!(found, Some(name), "{path}");
    }

    for path in unlisted {
        assert_eq!(listed_file(path), None, "{path}");
    }
}

#[test]
fn serves_the_libraries_only_by_their_versioned_names() {
    for file in LIBRARIES {
        for path in [
            format!("/{}", file.name),
            format!("/thor-tigress-cub/{}", file.name),
        ] {
            assert_eq!(listed_file(&path), Some(file), "{path}");
        }
    }

    for path in [
        "/mermaid.min.js",
        "/katex.min.js",
        "/node_modules/katex/dist/katex.min.js",
    ] {
        assert_eq!(listed_file(path), None, "{path}");
    }
}

#[tokio::test]
async fn serves_the_page_and_health_without_a_key() -> Result<()> {
    let web = web_folder()?;
    let config = setup(web.path(), Vec::new(), Some("k"))?;
    let page = ask(&config, get("/")).await?;
    let style = ask(&config, get("/thor-tigress-cub/chat.css")).await?;
    let health = ask(&config, get("/health")).await?;
    let html = "text/html; charset=utf-8";

    assert_eq!(
        (page.status, page.body.as_str()),
        (StatusCode::OK, "<p>cub</p>")
    );
    assert_eq!(page.header(header::CONTENT_TYPE), html);
    assert_eq!(page.header(header::ACCESS_CONTROL_ALLOW_ORIGIN), "*");
    assert_eq!(page.header(header::CACHE_CONTROL), "no-store");
    assert_eq!(
        style.header(header::CONTENT_TYPE),
        "text/css; charset=utf-8"
    );
    assert_eq!(health.body, r#"{"status":"ok"}"#);

    Ok(())
}

#[tokio::test]
async fn a_library_may_be_kept_and_a_page_file_may_not() -> Result<()> {
    let web = web_folder()?;
    fs::write(web.path().join("pdf-6.4.299-legacy.min.mjs"), "export {}")?;

    let config = setup(web.path(), Vec::new(), Some("k"))?;
    let library = ask(&config, get("/thor-tigress-cub/pdf-6.4.299-legacy.min.mjs")).await?;
    let style = ask(&config, get("/chat.css")).await?;

    assert_eq!(library.status, StatusCode::OK);
    assert_eq!(
        library.header(header::CONTENT_TYPE),
        "text/javascript; charset=utf-8",
        "a module script must be served as JavaScript or the browser refuses it"
    );
    assert_eq!(library.header(header::CACHE_CONTROL), KEPT);
    assert_eq!(style.header(header::CACHE_CONTROL), "no-store");

    Ok(())
}

#[tokio::test]
async fn a_missing_file_is_an_error_and_anything_else_is_not_found() -> Result<()> {
    let web = web_folder()?;
    let config = setup(web.path(), Vec::new(), Some("k"))?;
    let missing = ask(&config, get("/about.html")).await?;
    let unlisted = ask(&config, get("/serve.sh")).await?;
    let posted = ask(&config, post("/health", "")).await?;
    let posted_page = ask(&config, post("/", "")).await?;

    assert_eq!(
        (missing.status, missing.body.as_str()),
        (StatusCode::INTERNAL_SERVER_ERROR, ""),
        "a listed file missing from the folder is an I/O error"
    );

    for reply in [unlisted, posted, posted_page] {
        assert_eq!(
            (reply.status, reply.body.as_str()),
            (StatusCode::NOT_FOUND, "not found")
        );
    }

    Ok(())
}

#[tokio::test]
async fn the_model_needs_the_key_but_preflight_does_not() -> Result<()> {
    let web = web_folder()?;
    let config = setup(web.path(), Vec::new(), Some("k"))?;
    let wrong_key = [("authorization", "Bearer x")];
    let refused = ask(&config, get("/v1/models")).await?;
    let wrong = ask(&config, get("/v1/embeddings").with(&wrong_key)).await?;
    let preflight = Asked {
        method: "OPTIONS",
        ..get("/v1/models")
    };
    let allowed = ask(&config, preflight).await?;

    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert_eq!(refused.body, INVALID_KEY);
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    assert_eq!(
        allowed.header(header::ACCESS_CONTROL_ALLOW_HEADERS),
        "Authorization, Content-Type, x-api-key, anthropic-version"
    );
    assert_eq!(allowed.header(header::ACCESS_CONTROL_ALLOW_ORIGIN), "*");

    Ok(())
}

/// A server whose first engine is `default` and whose second holds `qwen`.
fn with_qwen(default: &FakeServer, qwen: &FakeServer) -> Result<Arc<Config>> {
    let mut config = Config::from_args([
        "--engine".to_string(),
        format!("nemotron={}", default.address()),
        "--key-file".to_string(),
        "/nonexistent".to_string(),
    ])?;

    config
        .upstreams
        .others
        .push(Engine::edge("qwen".to_string(), qwen.address()));

    Ok(Arc::new(config))
}

#[tokio::test]
async fn sends_requests_for_an_engine_model_to_that_engine() -> Result<()> {
    let reply = json_response(r#"{"ok":true}"#);
    let engine = FakeServer::start(vec![reply.clone(), reply.clone(), reply])?;
    let config = with_qwen(&FakeServer::start(Vec::new())?, &engine)?;
    let chatted = ask(&config, post(paths::CHAT_COMPLETIONS, QUESTION)).await?;
    let forwarded = ask(&config, post(paths::MESSAGES, QUESTION)).await?;
    let counted = ask(&config, post(paths::COUNT_TOKENS, QUESTION)).await?;
    let seen = engine.requests()?;

    for reply in [chatted, forwarded, counted] {
        assert_eq!(
            (reply.status, reply.body.as_str()),
            (StatusCode::OK, r#"{"ok":true}"#)
        );
        assert_eq!(reply.header(header::CONTENT_TYPE), "application/json");
    }

    assert!(seen[0].starts_with("POST /v1/chat/completions"));
    assert!(seen[1].starts_with("POST /v1/messages "));
    assert!(seen[2].starts_with("POST /v1/messages/count_tokens"));

    Ok(())
}

#[tokio::test]
async fn answers_model_routes_from_the_engine_for_a_key() -> Result<()> {
    let replies = vec![
        json_response(r#"{"data":[]}"#),
        json_response(r#"{"input_tokens":3}"#),
    ];
    let web = web_folder()?;
    let config = setup(web.path(), replies, Some("k"))?;
    let bearer = [("authorization", "Bearer k")];
    let api_key = [("x-api-key", "k")];
    let models = ask(&config, get("/v1/models").with(&bearer)).await?;
    let counted = post("/v1/messages/count_tokens", "{}").with(&api_key);
    let count = ask(&config, counted).await?;
    let unknown = ask(&config, get("/v1/embeddings").with(&bearer)).await?;
    let not_json = ask(&config, post("/v1/messages", "{x").with(&bearer)).await?;

    assert_eq!(models.body, r#"{"data":[],"object":"list"}"#);
    assert_eq!(count.body, r#"{"input_tokens":3}"#);
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(not_json.status, StatusCode::BAD_REQUEST);

    Ok(())
}

const KEYRING_PASS: &str = "keyring passphrase";

/// A server that keeps a new, empty keyring in `folder`.
fn with_keyring(folder: &Path) -> Result<Arc<Config>> {
    let keyring = folder.join("keyring");
    let passphrase = folder.join("passphrase");

    fs::write(&passphrase, format!("{KEYRING_PASS}\n"))?;
    Keyring::create(&keyring, KEYRING_PASS)?;

    let arguments = [
        "--web".to_string(),
        folder.display().to_string(),
        "--engine".to_string(),
        "nemotron=127.0.0.1:1".to_string(),
        "--key-file".to_string(),
        "/nonexistent".to_string(),
        "--keyring".to_string(),
        keyring.display().to_string(),
        "--keyring-passphrase-file".to_string(),
        passphrase.display().to_string(),
    ];

    Ok(Arc::new(Config::from_args(arguments)?))
}

#[tokio::test]
async fn the_registration_form_needs_no_key_and_records_a_request() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let config = with_keyring(folder.path())?;
    let form = r#"{"name":"Ada Lovelace","email":"ada@example.com"}"#;
    let no_email = r#"{"name":"Ada","email":"not-an-address"}"#;
    let recorded = ask(&config, post("/request", form)).await?;
    let not_json = ask(&config, post("/request", "not json")).await?;
    let refused = ask(&config, post("/request", no_email)).await?;
    let reopened = Keyring::open(folder.path().join("keyring"), KEYRING_PASS)?;

    assert_eq!(
        (recorded.status, recorded.body.as_str()),
        (StatusCode::OK, r#"{"status":"recorded"}"#)
    );
    assert!(reopened.find("ada@example.com").is_some());
    assert_eq!(not_json.status, StatusCode::BAD_REQUEST);
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn the_registration_form_is_closed_without_a_keyring() -> Result<()> {
    let web = web_folder()?;
    let closed = setup(web.path(), Vec::new(), Some("k"))?;
    let form = r#"{"name":"Ada Lovelace","email":"ada@example.com"}"#;
    let unkept = ask(&closed, post("/request", form)).await?;

    assert_eq!(unkept.status, StatusCode::BAD_REQUEST);

    Ok(())
}
