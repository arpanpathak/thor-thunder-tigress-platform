//! `/v1/models`: the models that can answer right now, one list made from
//! every engine's own, so the chat page's picker offers exactly those.

use std::io::Write;

use serde_json::{Value, json};

use crate::{
    config::Upstreams,
    error::Outcome,
    paths,
    response::{self, ContentType, Status},
    upstream::Endpoint,
};

/// Answers `/v1/models`. An engine that doesn't answer is left out, so one
/// engine being down never hides the others.
///
/// # Errors
///
/// `AgentError::Io` when the client is gone.
pub fn list(client: &mut dyn Write, upstreams: &Upstreams) -> Outcome {
    let data: Vec<Value> = upstreams
        .engines()
        .filter_map(|engine| listed(&engine.endpoint))
        .flatten()
        .collect();
    let list = json!({ "object": "list", "data": data });

    response::respond(
        client,
        Status::Ok,
        ContentType::Json,
        &serde_json::to_vec(&list)?,
    )
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

    fn upstreams(first: String, others: &[String]) -> Upstreams {
        let engine = |address: &String| Engine::new("m".to_string(), address.clone());

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
}
