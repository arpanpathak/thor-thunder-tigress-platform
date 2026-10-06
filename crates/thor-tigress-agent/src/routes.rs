//! Which request goes where: one `Route` per thing the server does, and one
//! function that answers each.

use std::{fs, io::Write, path::Path};

use crate::{
    chat,
    config::Config,
    error::Outcome,
    messages, paths,
    request::Request,
    response::{self, ContentType, Status},
};

/// What `/health` answers.
const HEALTHY: &[u8] = br#"{"status":"ok"}"#;

/// A file the server hands out, with its content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticFile {
    name: &'static str,
    content_type: ContentType,
}

/// The chat page.
const PAGE: StaticFile = StaticFile { name: "index.html", content_type: ContentType::Html };

/// The other files in the web folder that may be served; nothing else is.
const FILES: [StaticFile; 3] = [
    StaticFile { name: "about.html", content_type: ContentType::Html },
    StaticFile { name: "cub.svg", content_type: ContentType::Svg },
    StaticFile { name: "cub.png", content_type: ContentType::Png },
];

/// What a request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// `OPTIONS` on any path: a browser's CORS preflight.
    Preflight,
    /// One of the listed files, the chat page included.
    File(StaticFile),
    /// `GET /health`.
    Health,
    /// `GET /v1/models`.
    Models,
    /// `POST /v1/chat/completions`.
    ChatCompletions,
    /// `POST /v1/messages`, Anthropic's API.
    Messages,
    /// `POST /v1/messages/count_tokens`.
    CountTokens,
    /// Anything else.
    NotFound,
}

impl Route {
    /// The route for `method` and `path`.
    #[must_use]
    pub fn of(method: &str, path: &str) -> Route {
        match (method, path) {
            ("OPTIONS", _) => Route::Preflight,
            ("GET", path) if is_page(path) => Route::File(PAGE),
            ("GET", paths::HEALTH) => Route::Health,
            ("GET", paths::MODELS) => Route::Models,
            ("GET", path) => listed_file(path).map_or(Route::NotFound, Route::File),
            ("POST", paths::CHAT_COMPLETIONS) => Route::ChatCompletions,
            ("POST", paths::MESSAGES) => Route::Messages,
            ("POST", paths::COUNT_TOKENS) => Route::CountTokens,
            _ => Route::NotFound,
        }
    }
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

/// The listed file a path names, at the root or under `/thor-tigress-cub/`.
fn listed_file(path: &str) -> Option<StaticFile> {
    let name = within_site(path).strip_prefix('/')?;
    FILES.into_iter().find(|file| file.name == name)
}

/// Answers `request`: refuses it without the key when it touches the model
/// (`/v1/…`), and otherwise does what its route asks.
///
/// # Errors
///
/// Whatever the route's handler returns: I/O, upstream, JSON or bad-request
/// errors.
pub fn answer(client: &mut dyn Write, request: &Request, config: &Config) -> Outcome {
    let route = Route::of(&request.method, &request.path);
    let needs_key = route != Route::Preflight && request.path.starts_with(paths::API);
    if needs_key && !config.admits(request.authorization.as_deref()) {
        return response::unauthorized(client);
    }
    let model = &config.upstreams.model;
    match route {
        Route::Preflight => response::preflight(client),
        Route::File(file) => send_file(client, &config.web, file),
        Route::Health => response::respond(client, Status::Ok, ContentType::Json, HEALTHY),
        Route::Models => model.get(paths::MODELS)?.relay(client),
        Route::ChatCompletions => chat::answer(client, &request.body, &config.upstreams),
        Route::Messages => messages::forward(client, &request.body, model),
        Route::CountTokens => model.post(paths::COUNT_TOKENS, &request.body)?.relay(client),
        Route::NotFound => response::respond(client, Status::NotFound, ContentType::Text, b"not found"),
    }
}

fn send_file(client: &mut dyn Write, folder: &Path, file: StaticFile) -> Outcome {
    let body = fs::read(folder.join(file.name))?;
    response::respond(client, Status::Ok, file.content_type, &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, json_response};

    fn file(name: &str) -> Route {
        FILES.into_iter().find(|file| file.name == name).map_or(Route::NotFound, Route::File)
    }

    #[test]
    fn routes_every_path() {
        let table = [
            ("OPTIONS", "/v1/chat/completions", Route::Preflight),
            ("GET", "/", Route::File(PAGE)),
            ("GET", "/thor-tigress-cub/", Route::File(PAGE)),
            ("GET", "/thor-tigress-cub", Route::File(PAGE)),
            ("GET", "/index.html", Route::File(PAGE)),
            ("GET", "/health", Route::Health),
            ("GET", "/v1/models", Route::Models),
            ("POST", "/v1/chat/completions", Route::ChatCompletions),
            ("POST", "/v1/messages", Route::Messages),
            ("POST", "/v1/messages/count_tokens", Route::CountTokens),
            ("GET", "/about.html", file("about.html")),
            ("GET", "/thor-tigress-cub/cub.svg", file("cub.svg")),
            ("GET", "/cub.png", file("cub.png")),
            ("POST", "/health", Route::NotFound),
            ("GET", "/v1/embeddings", Route::NotFound),
        ];
        for (method, path, route) in table {
            assert_eq!(Route::of(method, path), route, "{method} {path}");
        }
    }

    #[test]
    fn serves_only_the_listed_files() {
        for path in ["/serve.sh", "/../../.config/thor-chat/api-key", "/thor-tigress-cub/../serve.sh", "about.html", "/"] {
            assert_eq!(listed_file(path), None, "{path}");
        }
    }

    struct Setup {
        config: Config,
        model: FakeServer,
        folder: std::path::PathBuf,
    }

    fn setup(responses: Vec<String>, key: Option<&str>) -> Outcome<Setup> {
        let folder = std::env::temp_dir().join(format!("thor-web-{}-{}", std::process::id(), rand_suffix()));
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("index.html"), "<p>cub</p>")?;
        let model = FakeServer::start(responses)?;
        let mut config = Config::from_args([
            "--web".to_string(),
            folder.to_string_lossy().into_owned(),
            "--model".to_string(),
            model.address(),
            "--key-file".to_string(),
            "/nonexistent".to_string(),
        ])?;
        config.key = key.map(ToString::to_string);
        Ok(Setup { config, model, folder })
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    }

    fn request(method: &str, path: &str, authorization: Option<&str>) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            authorization: authorization.map(ToString::to_string),
            body: b"{}".to_vec(),
        }
    }

    fn answered(setup: &Setup, request: &Request) -> Outcome<String> {
        let mut client = Vec::new();
        answer(&mut client, request, &setup.config)?;
        Ok(String::from_utf8_lossy(&client).into_owned())
    }

    #[test]
    fn serves_the_page_and_health_without_a_key() -> Outcome {
        let setup = setup(Vec::new(), Some("k"))?;
        let page = answered(&setup, &request("GET", "/", None))?;
        let health = answered(&setup, &request("GET", "/health", None))?;
        let missing = answered(&setup, &request("GET", "/about.html", None));
        fs::remove_dir_all(&setup.folder)?;
        assert!(page.starts_with("HTTP/1.1 200 OK") && page.ends_with("<p>cub</p>"));
        assert!(health.ends_with(r#"{"status":"ok"}"#));
        assert!(missing.is_err(), "a listed file missing from the folder is an I/O error");
        Ok(())
    }

    #[test]
    fn the_model_needs_the_key_but_preflight_does_not() -> Outcome {
        let setup = setup(Vec::new(), Some("k"))?;
        let refused = answered(&setup, &request("GET", "/v1/models", None))?;
        let wrong = answered(&setup, &request("GET", "/v1/embeddings", Some("Bearer x")))?;
        let preflight = answered(&setup, &request("OPTIONS", "/v1/models", None))?;
        fs::remove_dir_all(&setup.folder)?;
        assert!(refused.starts_with("HTTP/1.1 401 Unauthorized"));
        assert!(wrong.starts_with("HTTP/1.1 401 Unauthorized"));
        assert!(preflight.starts_with("HTTP/1.1 204 No Content"));
        Ok(())
    }

    #[test]
    fn sends_chat_and_messages_to_the_model() -> Outcome {
        let reply = json_response(r#"{"choices":[{"message":{"role":"assistant","content":"hi"}}]}"#);
        let setup = setup(vec![reply.clone(), reply], None)?;
        let mut chat = request("POST", "/v1/chat/completions", None);
        chat.body = br#"{"messages":[{"role":"user","content":"hi"}]}"#.to_vec();
        let mut messages = request("POST", "/v1/messages", None);
        messages.body = br#"{"model":"x","messages":[{"role":"user","content":"hi"}]}"#.to_vec();
        let chatted = answered(&setup, &chat)?;
        let forwarded = answered(&setup, &messages)?;
        fs::remove_dir_all(&setup.folder)?;
        assert!(chatted.starts_with("HTTP/1.1 200"), "{chatted}");
        assert!(forwarded.starts_with("HTTP/1.1 200"), "{forwarded}");
        Ok(())
    }

    #[test]
    fn passes_model_routes_through_with_the_key() -> Outcome {
        let setup = setup(vec![json_response(r#"{"data":[]}"#), json_response(r#"{"input_tokens":3}"#)], Some("k"))?;
        let models = answered(&setup, &request("GET", "/v1/models", Some("Bearer k")))?;
        let count = answered(&setup, &request("POST", "/v1/messages/count_tokens", Some("Bearer k")))?;
        let unknown = answered(&setup, &request("GET", "/v1/embeddings", Some("Bearer k")))?;
        fs::remove_dir_all(&setup.folder)?;
        let seen = setup.model.requests()?;
        assert!(models.ends_with(r#"{"data":[]}"#) && count.ends_with(r#"{"input_tokens":3}"#));
        assert!(unknown.starts_with("HTTP/1.1 404 Not Found"));
        assert!(seen[0].starts_with("GET /v1/models") && seen[1].starts_with("POST /v1/messages/count_tokens"));
        Ok(())
    }
}
