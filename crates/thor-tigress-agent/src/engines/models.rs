//! `/v1/models`: the models that can answer right now, one list made from
//! every server's own, so the chat page's picker offers exactly those.

use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::{
    config::{Engine, Upstreams},
    error::Outcome,
    paths,
    server::response::{self, Client, JSON},
    engines::upstream::Endpoint,
};

/// What llama-server in router mode calls a model it is holding in memory.
const LOADED: &str = "loaded";

/// Answers `/v1/models`. A server that doesn't answer is left out, so one
/// being down never hides the others.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn list(client: &mut dyn Client, upstreams: &Upstreams) -> Outcome {
    let data: Vec<Value> = upstreams
        .engines()
        .flat_map(offered)
        .map(with_context)
        .collect();
    let list = json!({ "object": "list", "data": data });

    response::respond(client, StatusCode::OK, JSON, &serde_json::to_vec(&list)?)
}

/// The models a server can answer for now: everything an Edge-LLM server
/// lists, and of llama-server's list only what it has loaded.
fn offered(engine: &Engine) -> Vec<Value> {
    let listed = listed(engine.endpoint()).unwrap_or_default();

    match engine {
        Engine::Edge { .. } => listed,
        Engine::Llama { .. } => listed.into_iter().filter(ready).collect(),
    }
}

/// Publishes llama.cpp's configured context in the field coding clients read.
/// The training context can be larger than the memory reserved by the server.
fn with_context(mut model: Value) -> Value {
    let context = model.pointer("/meta/n_ctx").and_then(Value::as_u64);
    let parallel = model
        .pointer("/status/args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|pair| pair[0].as_str() == Some("--parallel"))
        .and_then(|pair| pair[1].as_str())
        .and_then(|count| count.parse::<u64>().ok())
        .filter(|count| *count > 0)
        .unwrap_or(1);

    if model.get("max_model_len").is_none()
        && let Some(context) = context.filter(|context| *context >= parallel)
    {
        model["max_model_len"] = json!(context / parallel);
    }

    model
}

/// The names llama-server answers to right now: the id and the aliases of
/// each model it has loaded. Empty when it can't be reached.
pub fn loaded_names(llama: &Endpoint) -> Vec<String> {
    listed(llama)
        .unwrap_or_default()
        .iter()
        .filter(|model| ready(model))
        .flat_map(names)
        .collect()
}

/// Every name llama-server or vLLM lists, loaded or not: a name it lists can
/// be asked for, and llama-server's router loads it on that request.
pub fn listed_names(server: &Endpoint) -> Vec<String> {
    listed(server)
        .unwrap_or_default()
        .iter()
        .flat_map(names)
        .collect()
}

/// A model's id and its aliases.
fn names(model: &Value) -> Vec<String> {
    let aliases = model.get("aliases").and_then(Value::as_array);

    model
        .get("id")
        .into_iter()
        .chain(aliases.into_iter().flatten())
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect()
}

/// Whether llama-server can serve a model without loading it.
///
/// In router mode it lists its whole preset, loaded or not. A model that is
/// only on disk is left out: offering it would send a request to a model that
/// is not there. A server that says nothing about a model's state is taken at
/// its word.
fn ready(model: &Value) -> bool {
    match model.pointer("/status/value").and_then(Value::as_str) {
        Some(state) => state == LOADED,
        None => true,
    }
}

/// The models a server lists; `None` when it can't be reached or answers
/// with something else.
fn listed(server: &Endpoint) -> Option<Vec<Value>> {
    let text = server.get(paths::MODELS).ok()?.text().ok()?;
    let list: Value = serde_json::from_str(&text).ok()?;

    list.get("data")?.as_array().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, FakeWeb, json_response};

    #[test]
    fn reports_allocated_context_instead_of_training_context() {
        let model = with_context(json!({
            "meta": { "n_ctx": 65_536, "n_ctx_train": 262_144 },
            "context_length": 262_144,
            "status": { "args": ["--parallel", "1"] }
        }));

        assert_eq!(model["max_model_len"], 65_536);
    }

    #[test]
    fn divides_reserved_context_among_parallel_slots() {
        let model = with_context(json!({
            "meta": { "n_ctx": 131_072 },
            "status": { "args": ["--parallel", "4"] }
        }));

        assert_eq!(model["max_model_len"], 32_768);
    }

    #[test]
    fn preserves_upstream_context_and_leaves_unknown_context_unset() {
        let model = with_context(json!({ "max_model_len": 32_768, "meta": { "n_ctx": 65_536 } }));

        assert_eq!(model["max_model_len"], 32_768);
        assert!(
            with_context(json!({ "id": "unknown" }))
                .get("max_model_len")
                .is_none()
        );
    }

    fn upstreams(first: String, others: &[String]) -> Upstreams {
        let engine = |address: &String| Engine::edge("m".to_string(), address.clone());

        Upstreams {
            default: engine(&first),
            others: others.iter().map(engine).collect(),
            search: Endpoint::new(first, None),
            web: Box::new(FakeWeb::default()),
        }
    }

    /// An address nothing listens on.
    fn closed() -> Outcome<String> {
        Ok(std::net::TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .to_string())
    }

    fn listed_ids(client: &[u8]) -> Outcome<Vec<String>> {
        let text = String::from_utf8_lossy(client);
        let body = text.split("\r\n\r\n").nth(1).unwrap_or_default();
        let list: Value = serde_json::from_str(body)?;

        Ok(list["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|model| model["id"].as_str())
            .map(ToString::to_string)
            .collect())
    }

    #[test]
    fn lists_every_engine_that_answers_the_default_first() -> Outcome {
        let qwen = FakeServer::start(vec![json_response(r#"{"data":[{"id":"qwen"}]}"#)])?;
        let lightning = FakeServer::start(vec![json_response(r#"{"data":[{"id":"lightning"}]}"#)])?;
        let garbled = FakeServer::start(vec![json_response("not json")])?;
        let empty = FakeServer::start(vec![json_response(r#"{"object":"list"}"#)])?;
        let others = [
            closed()?,
            garbled.address(),
            lightning.address(),
            empty.address(),
        ];
        let mut client = Vec::new();

        list(&mut client, &upstreams(qwen.address(), &others))?;

        assert_eq!(listed_ids(&client)?, ["qwen", "lightning"]);

        Ok(())
    }

    #[test]
    fn no_engine_answering_is_an_empty_list() -> Outcome {
        let mut client = Vec::new();

        list(&mut client, &upstreams(closed()?, &[closed()?]))?;

        let text = String::from_utf8_lossy(&client);

        assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
        assert!(listed_ids(&client)?.is_empty());

        Ok(())
    }

    /// What llama-server in router mode lists: its whole preset, with a state.
    const PRESET: &str = r#"{"data":[
        {"id":"nano","aliases":["nemotron","nemotron-think"],"status":{"value":"loaded"}},
        {"id":"on-disk","aliases":["other"],"status":{"value":"unloaded"}},
        {"id":"plain"}]}"#;

    #[test]
    fn llama_server_offers_only_what_it_has_loaded() -> Outcome {
        let llama = FakeServer::start(vec![json_response(PRESET)])?;
        let edge = FakeServer::start(vec![json_response(r#"{"data":[{"id":"qwen"}]}"#)])?;
        let mut both = upstreams(edge.address(), &[]);
        both.others.push(Engine::llama(llama.address(), None));
        let mut client = Vec::new();

        list(&mut client, &both)?;

        assert_eq!(listed_ids(&client)?, ["qwen", "nano", "plain"]);

        Ok(())
    }

    #[test]
    fn llama_server_answers_to_the_ids_and_aliases_of_loaded_models() -> Outcome {
        let llama = FakeServer::start(vec![json_response(PRESET)])?;

        let names = loaded_names(&Endpoint::new(llama.address(), None));

        assert_eq!(names, ["nano", "nemotron", "nemotron-think", "plain"]);
        assert!(loaded_names(&Endpoint::new(closed()?, None)).is_empty());

        Ok(())
    }
}
