use std::fs;

use anyhow::Result;
use axum::body::{Body, to_bytes};
use tower::ServiceExt;

use super::*;
use crate::{
    config::Engine,
    testing::{FakeServer, json_response},
};

const QUESTION: &str = r#"{"model":"qwen","messages":[{"role":"user","content":"hi"}]}"#;

/// A server with a web folder, one fake engine and maybe a key.
struct Setup {
    config: Arc<Config>,
    model: FakeServer,
    folder: std::path::PathBuf,
}

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

fn folder(name: &str) -> Result<std::path::PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let folder = std::env::temp_dir().join(format!("thor-{name}-{}-{nanos}", std::process::id()));

    fs::create_dir_all(&folder)?;

    Ok(folder)
}

fn setup(responses: Vec<String>, key: Option<&str>) -> Result<Setup> {
    let folder = folder("web")?;

    fs::write(folder.join("index.html"), "<p>cub</p>")?;
    fs::write(folder.join("chat.css"), "p {}")?;

    let model = FakeServer::start(responses)?;
    let arguments = [
        "--web".to_string(),
        folder.to_string_lossy().into_owned(),
        "--engine".to_string(),
        format!("nemotron={}", model.address()),
        "--key-file".to_string(),
        "/nonexistent".to_string(),
    ];
    let mut config = Config::from_args(arguments)?;

    config.key = key.map(ToString::to_string);

    Ok(Setup {
        config: Arc::new(config),
        model,
        folder,
    })
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
fn serves_only_the_listed_files() {
    let pages = [
        "/",
        "/thor-tigress-cub/",
        "/thor-tigress-cub",
        "/index.html",
    ];
    let listed = [
        ("/about.html", "about.html"),
        ("/chat.css", "chat.css"),
        ("/thor-tigress-cub/chat.js", "chat.js"),
        ("/thor-tigress-cub/cub.svg", "cub.svg"),
        ("/cub.png", "cub.png"),
    ];
    let unlisted = [
        "/serve.sh",
        "/../../.config/thor-chat/api-key",
        "/thor-tigress-cub/../serve.sh",
        "about.html",
    ];

    for path in pages {
        assert_eq!(listed_file(path), Some(PAGE), "{path}");
    }

    for (path, name) in listed {
        assert_eq!(
            listed_file(path).map(|file| file.name),
            Some(name),
            "{path}"
        );
    }

    for path in unlisted {
        assert_eq!(listed_file(path), None, "{path}");
    }
}

#[tokio::test]
async fn serves_the_page_and_health_without_a_key() -> Result<()> {
    let setup = setup(Vec::new(), Some("k"))?;
    let page = ask(&setup.config, get("/")).await?;
    let style = ask(&setup.config, get("/thor-tigress-cub/chat.css")).await?;
    let health = ask(&setup.config, get("/health")).await?;
    let missing = ask(&setup.config, get("/about.html")).await?;
    let unlisted = ask(&setup.config, get("/serve.sh")).await?;
    let posted = ask(&setup.config, post("/health", "")).await?;
    let posted_page = ask(&setup.config, post("/", "")).await?;

    fs::remove_dir_all(&setup.folder)?;

    assert_eq!(
        (page.status, page.body.as_str()),
        (StatusCode::OK, "<p>cub</p>")
    );
    assert_eq!(
        page.header(header::CONTENT_TYPE),
        "text/html; charset=utf-8"
    );
    assert_eq!(page.header(header::ACCESS_CONTROL_ALLOW_ORIGIN), "*");
    assert_eq!(page.header(header::CACHE_CONTROL), "no-store");
    assert_eq!(
        style.header(header::CONTENT_TYPE),
        "text/css; charset=utf-8"
    );
    assert_eq!(health.body, r#"{"status":"ok"}"#);
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
    let setup = setup(Vec::new(), Some("k"))?;
    let wrong_key = [("authorization", "Bearer x")];
    let refused = ask(&setup.config, get("/v1/models")).await?;
    let wrong = ask(&setup.config, get("/v1/embeddings").with(&wrong_key)).await?;
    let preflight = Asked {
        method: "OPTIONS",
        ..get("/v1/models")
    };
    let allowed = ask(&setup.config, preflight).await?;

    fs::remove_dir_all(&setup.folder)?;

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

#[tokio::test]
async fn sends_requests_for_an_engine_model_to_that_engine() -> Result<()> {
    let reply = json_response(r#"{"ok":true}"#);
    let engine = FakeServer::start(vec![reply.clone(), reply.clone(), reply])?;
    let setup = setup(Vec::new(), None)?;
    let mut config = Config::from_args([
        "--engine".to_string(),
        format!("nemotron={}", setup.model.address()),
        "--key-file".to_string(),
        "/nonexistent".to_string(),
    ])?;

    config
        .upstreams
        .others
        .push(Engine::edge("qwen".to_string(), engine.address()));

    let config = Arc::new(config);
    let chatted = ask(&config, post(paths::CHAT_COMPLETIONS, QUESTION)).await?;
    let forwarded = ask(&config, post(paths::MESSAGES, QUESTION)).await?;
    let counted = ask(&config, post(paths::COUNT_TOKENS, QUESTION)).await?;

    fs::remove_dir_all(&setup.folder)?;

    let seen = engine.requests()?;

    for reply in [chatted, forwarded, counted] {
        assert_eq!(
            (reply.status, reply.body.as_str()),
            (StatusCode::OK, r#"{"ok":true}"#)
        );
        assert_eq!(reply.header(header::CONTENT_TYPE), "application/json");
    }

    assert!(
        seen[0].starts_with("POST /v1/chat/completions")
            && seen[1].starts_with("POST /v1/messages ")
    );
    assert!(seen[2].starts_with("POST /v1/messages/count_tokens"));

    Ok(())
}

#[tokio::test]
async fn answers_model_routes_from_the_engine_for_a_key() -> Result<()> {
    let replies = vec![
        json_response(r#"{"data":[]}"#),
        json_response(r#"{"input_tokens":3}"#),
    ];
    let setup = setup(replies, Some("k"))?;
    let bearer = [("authorization", "Bearer k")];
    let api_key = [("x-api-key", "k")];
    let models = ask(&setup.config, get("/v1/models").with(&bearer)).await?;
    let counted = post("/v1/messages/count_tokens", "{}").with(&api_key);
    let count = ask(&setup.config, counted).await?;
    let unknown = ask(&setup.config, get("/v1/embeddings").with(&bearer)).await?;
    let not_json = ask(&setup.config, post("/v1/messages", "{x").with(&bearer)).await?;

    fs::remove_dir_all(&setup.folder)?;

    assert_eq!(models.body, r#"{"data":[],"object":"list"}"#);
    assert_eq!(count.body, r#"{"input_tokens":3}"#);
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(not_json.status, StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn the_registration_form_needs_no_key_and_records_a_request() -> Result<()> {
    use thor_tigress_keyring::store::Keyring;

    let folder = folder("agent-request")?;
    let keyring = folder.join("keyring");
    let passphrase = folder.join("passphrase");

    fs::write(&passphrase, "keyring passphrase\n")?;
    Keyring::create(&keyring, "keyring passphrase")?;

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
    let config = Arc::new(Config::from_args(arguments)?);
    let form = r#"{"name":"Ada Lovelace","email":"ada@example.com"}"#;
    let recorded = ask(&config, post("/request", form)).await?;
    let not_json = ask(&config, post("/request", "not json")).await?;
    let no_email = r#"{"name":"Ada","email":"not-an-address"}"#;
    let refused = ask(&config, post("/request", no_email)).await?;
    let closed = setup(Vec::new(), Some("k"))?;
    let unkept = ask(&closed.config, post("/request", form)).await?;
    let reopened = Keyring::open(&keyring, "keyring passphrase")?;

    fs::remove_dir_all(&closed.folder)?;
    fs::remove_dir_all(&folder)?;

    assert_eq!(
        (recorded.status, recorded.body.as_str()),
        (StatusCode::OK, r#"{"status":"recorded"}"#)
    );
    assert!(reopened.find("ada@example.com").is_some());
    assert_eq!(not_json.status, StatusCode::BAD_REQUEST);
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(unkept.status, StatusCode::BAD_REQUEST);

    Ok(())
}
