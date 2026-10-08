//! `/v1/models`: what can answer right now -- llama-server's loaded models,
//! with the models of every other engine added, so the chat page's picker
//! offers exactly those.

use std::io::Write;

use serde_json::Value;

use crate::{
    config::Upstreams,
    error::Outcome,
    paths,
    response::{self, ContentType, Status},
    upstream::Endpoint,
};

/// What llama-server in router mode calls a model it is holding in memory.
const LOADED: &str = "loaded";

/// Answers `/v1/models`. An engine that doesn't answer is left out, so one
/// engine being down never hides the others. When llama-server answers with an
/// error, its answer is relayed as it is.
///
/// # Errors
///
/// Upstream and I/O errors from llama-server; `AgentError::Json` when its
/// list isn't JSON.
pub fn list(client: &mut dyn Write, upstreams: &Upstreams) -> Outcome {
    let answer = upstreams.model.get(paths::MODELS)?;

    if !answer.is_ok() {
        return answer.relay(client);
    }

    let mut list: Value = serde_json::from_str(&answer.text()?)?;

    if let Some(data) = list.get_mut("data").and_then(Value::as_array_mut) {
        data.retain(ready);
        data.extend(
            upstreams
                .engines
                .iter()
                .filter_map(|engine| listed(&engine.endpoint))
                .flatten(),
        );
    }
    response::respond(
        client,
        Status::Ok,
        ContentType::Json,
        &serde_json::to_vec(&list)?,
    )
}

/// Whether llama-server is offering a model it can serve without loading it.
///
/// In router mode it lists its whole preset, loaded or not, and loads one on
/// the first request that names it. A preset model that is only on disk is
/// therefore left out of the list: offering it would let the picker pull tens
/// of gigabytes back into memory, which is the thing the unload was for. A
/// server that says nothing about a model's state is taken at its word.
fn ready(model: &Value) -> bool {
    match model.pointer("/status/value").and_then(Value::as_str) {
        Some(state) => state == LOADED,
        None => true,
    }
}

/// The models an engine lists; `None` when it can't be reached or answers
/// with something else.
fn listed(engine: &Endpoint) -> Option<Vec<Value>> {
    let text = engine.get(paths::MODELS).ok()?.text().ok()?;
    let list: Value = serde_json::from_str(&text).ok()?;
    list.get("data")?.as_array().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Engine,
        testing::{FakeServer, FakeWeb, json_response},
    };

    fn upstreams(model: &FakeServer, engines: &[String]) -> Upstreams {
        Upstreams {
            model: Endpoint::new(model.address(), None),
            engines: engines
                .iter()
                .map(|address| Engine {
                    model: "m".to_string(),
                    endpoint: Endpoint::new(address.clone(), None),
                })
                .collect(),
            search: Endpoint::new(model.address(), None),
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
    fn adds_every_engine_that_answers() -> Outcome {
        let llama = FakeServer::start(vec![json_response(r#"{"data":[{"id":"nano"}]}"#)])?;
        let edge = FakeServer::start(vec![json_response(r#"{"data":[{"id":"qwen"}]}"#)])?;
        let garbled = FakeServer::start(vec![json_response("not json")])?;
        let empty = FakeServer::start(vec![json_response(r#"{"object":"list"}"#)])?;
        let engines = [
            edge.address(),
            closed()?,
            garbled.address(),
            empty.address(),
        ];
        let mut client = Vec::new();
        list(&mut client, &upstreams(&llama, &engines))?;
        assert_eq!(listed_ids(&client)?, ["nano", "qwen"]);
        Ok(())
    }

    #[test]
    fn relays_an_error_and_reads_a_list_it_can_parse() -> Outcome {
        let alone = FakeServer::start(vec![json_response(r#"{"data":[{"id":"nano"}]}"#)])?;
        let busy = "HTTP/1.1 503 Busy\r\nContent-Length: 4\r\n\r\nbusy".to_string();
        let failing = FakeServer::start(vec![busy])?;
        let mut relayed = Vec::new();
        let mut refused = Vec::new();
        list(&mut relayed, &upstreams(&alone, &[]))?;
        list(&mut refused, &upstreams(&failing, &[closed()?]))?;
        assert_eq!(listed_ids(&relayed)?, ["nano"]);
        assert!(String::from_utf8_lossy(&refused).starts_with("HTTP/1.1 503"));
        Ok(())
    }

    /// What llama-server in router mode does with a preset it isn't holding:
    /// it lists the model anyway, and would load it on the first request that
    /// names it. Offering it in the picker is how 35 GB comes back.
    #[test]
    fn leaves_out_a_model_llama_server_has_not_loaded() -> Outcome {
        let llama = FakeServer::start(vec![json_response(
            r#"{"data":[{"id":"in-memory","status":{"value":"loaded"}},
                        {"id":"only-on-disk","status":{"value":"unloaded"}}]}"#,
        )])?;
        let mut client = Vec::new();
        list(&mut client, &upstreams(&llama, &[]))?;
        assert_eq!(listed_ids(&client)?, ["in-memory"]);
        Ok(())
    }

    #[test]
    fn an_engine_is_still_added_beside_a_model_that_is_only_on_disk() -> Outcome {
        let llama = FakeServer::start(vec![json_response(
            r#"{"data":[{"id":"only-on-disk","status":{"value":"unloaded"}}]}"#,
        )])?;
        let edge = FakeServer::start(vec![json_response(r#"{"data":[{"id":"qwen"}]}"#)])?;
        let mut client = Vec::new();
        list(&mut client, &upstreams(&llama, &[edge.address()]))?;
        assert_eq!(listed_ids(&client)?, ["qwen"]);
        Ok(())
    }

    #[test]
    fn leaves_a_list_without_data_as_it_is() -> Outcome {
        let llama = FakeServer::start(vec![json_response(r#"{"models":[]}"#)])?;
        let edge = FakeServer::start(vec![json_response(r#"{"data":[{"id":"qwen"}]}"#)])?;
        let mut client = Vec::new();
        list(&mut client, &upstreams(&llama, &[edge.address()]))?;
        assert!(String::from_utf8_lossy(&client).ends_with(r#"{"models":[]}"#));
        Ok(())
    }

    #[test]
    fn a_list_that_is_not_json_is_an_error() -> Outcome {
        let llama = FakeServer::start(vec![json_response("not json")])?;
        let mut client = Vec::new();
        assert!(list(&mut client, &upstreams(&llama, &[closed()?])).is_err());
        Ok(())
    }
}
