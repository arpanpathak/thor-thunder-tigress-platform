//! The routes, asked through the router as the page asks them.

use std::sync::Arc;

use anyhow::Result;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
};
use tower::ServiceExt;

use super::{
    records::{MAX_INLINE_RESPONSE, shorten_at_line},
    router, serve,
};
use crate::{
    datasets::{
        app::App,
        workspace::{DatasetSpec, Workspace},
    },
    testing::TempDir,
};

const TRAINING: &str = concat!(
    r#"{"id":"a1","source":"chat","origin":"c1","instruction":"Why?","response":"Great question! Here's the thing: no."}"#,
    "\n",
    r#"{"id":"b2","source":"corpus","origin":"trpl/src/ch01.md","instruction":"What is a page?","response":"Here's The Thing about pages."}"#,
    "\n",
    r#"{"id":"c3","source":"readability","origin":"r","instruction":"Rewrite","response":"line one\nline two\nline three"}"#,
    "\n"
);

const FLAGS: &str = concat!(
    r#"{"id":"gone","note":"removed"}"#,
    "\n",
    r#"{"id":"lost","note":"text changed","spans":[]}"#,
    "\n",
    r#"{"id":"also-lost","note":"rebuilt","spans":[]}"#,
    "\n",
    r#"{"id":"auto-only","note":"auto: x","spans":[]}"#,
    "\n"
);

const TEACHER: &str = concat!(
    r#"{"id":"t1","source":"teacher","origin":"trpl/src/ch08.md","entry":"grounded/trpl.md#1","based_on":"trpl/src/ch08.md","licence":"Apache-2.0","source_text":"Vectors hold values.","notes":["turn 2: R1"],"messages":[{"role":"user","content":"Q?"},{"role":"assistant","content":"A."}]}"#,
    "\n"
);

/// Two datasets in a folder that lasts as long as the test.
struct Setup {
    workspace: Arc<Workspace>,
    _folder: TempDir,
}

impl Setup {
    /// `train` with `training` as its records, and the teacher set.
    fn with(training: &str) -> Result<Setup> {
        let folder = TempDir::new()?;
        let train = folder.file("train.jsonl", training)?;
        let teacher = folder.file("teacher.jsonl", TEACHER)?;
        let flags = folder.file("flags.jsonl", FLAGS)?;
        let slop = folder.file("slop.jsonl", "{\"id\":\"gone\",\"response\":\"old\"}\n")?;
        let specs = [
            DatasetSpec::new("train", &train, &flags, &slop),
            DatasetSpec::new(
                "teacher",
                &teacher,
                &folder.path().join("f2.jsonl"),
                &folder.path().join("r2.jsonl"),
            ),
        ];

        Ok(Setup {
            workspace: Arc::new(Workspace::open(&specs)?),
            _folder: folder,
        })
    }

    fn new() -> Result<Setup> {
        Setup::with(TRAINING)
    }

    /// The training set.
    fn app(&self) -> Result<&App> {
        Ok(&self.workspace.dataset(None)?.app)
    }

    /// Asks the router, and reads the whole answer.
    async fn call(&self, method: &str, target: &str, body: &str) -> Result<Reply> {
        let request = Request::builder()
            .method(method)
            .uri(target)
            .body(Body::from(body.to_string()))?;
        let response = router(Arc::clone(&self.workspace)).oneshot(request).await?;
        let status = response.status();
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string()
        };
        let (content_type, cache) = (header(header::CONTENT_TYPE), header(header::CACHE_CONTROL));
        let bytes = to_bytes(response.into_body(), usize::MAX).await?;
        let text = String::from_utf8_lossy(&bytes).into_owned();

        Ok(Reply {
            status,
            content_type,
            cache,
            body: serde_json::from_str(&text).unwrap_or(Value::String(text)),
        })
    }

    async fn get(&self, target: &str) -> Result<Reply> {
        self.call("GET", target, "").await
    }

    async fn post(&self, target: &str, body: &str) -> Result<Reply> {
        self.call("POST", target, body).await
    }
}

/// An answer: its status, two of its headers, and its body, as JSON when it is JSON.
struct Reply {
    status: StatusCode,
    content_type: String,
    cache: String,
    body: Value,
}

#[tokio::test]
async fn serves_the_page_the_dataset_list_and_each_dataset() -> Result<()> {
    let setup = Setup::new()?;
    let page = setup.get("/").await?;

    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.content_type, "text/html; charset=utf-8");
    assert_eq!(page.cache, "no-store");

    let listed = setup.get("/api/datasets").await?.body;

    assert_eq!(listed["datasets"][1]["name"], "teacher");
    assert_eq!(listed["missing"], json!([]));

    let item = &setup.get("/api/page?dataset=teacher").await?.body["items"][0];

    assert_eq!(item["source_text"], "Vectors hold values.");
    assert_eq!(item["licence"], "Apache-2.0");
    assert_eq!(item["collection_title"], "The Rust Programming Language");

    Ok(())
}

#[tokio::test]
async fn an_unknown_path_method_or_dataset_is_not_found() -> Result<()> {
    let setup = Setup::new()?;
    let path = setup.get("/missing").await?;
    let method = setup.call("DELETE", "/api/flag", "").await?;
    let dataset = setup.get("/api/meta?dataset=nope").await?;

    assert_eq!(path.status, StatusCode::NOT_FOUND);
    assert_eq!(path.body, json!({ "error": "not found: GET /missing" }));
    assert_eq!(
        method.body,
        json!({ "error": "not found: DELETE /api/flag" })
    );
    assert_eq!(dataset.body, json!({ "error": "not found: dataset nope" }));

    Ok(())
}

#[tokio::test]
async fn serves_the_removed_examples() -> Result<()> {
    let setup = Setup::new()?;
    let removed = setup.get("/slop.jsonl").await?;
    let none = setup.get("/slop.jsonl?dataset=teacher").await?;

    assert_eq!(removed.status, StatusCode::OK);
    assert_eq!(removed.content_type, "application/x-ndjson; charset=utf-8");
    assert_eq!(removed.body["id"], "gone");
    assert_eq!(none.body, "");

    Ok(())
}

#[tokio::test]
async fn meta_counts_sources_books_and_flags() -> Result<()> {
    let setup = Setup::new()?;
    let meta = setup.get("/api/meta").await?.body;

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

#[tokio::test]
async fn orphans_are_reviewed_flags_with_no_example() -> Result<()> {
    let setup = Setup::new()?;
    let orphans = setup.get("/api/orphans").await?.body;
    let expected = json!({ "orphans": [
        { "id": "also-lost", "note": "rebuilt", "spans": [] },
        { "id": "lost", "note": "text changed", "spans": [] }
    ] });

    assert_eq!(orphans, expected);

    Ok(())
}

#[tokio::test]
async fn categories_carry_names_and_words() -> Result<()> {
    let setup = Setup::new()?;
    let body = setup.get("/api/categories").await?.body;

    assert_eq!(
        body["categories"][0],
        json!({ "name": "fake_importance", "description": "fake importance" })
    );
    assert_eq!(body["categories"].as_array().map(Vec::len), Some(8));

    Ok(())
}

#[tokio::test]
async fn pages_filter_and_suggest() -> Result<()> {
    let setup = Setup::new()?;
    let page = setup.get("/api/page?source=chat&limit=500").await?.body;

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

    let searched = setup.get("/api/page?q=here%27s+the+thing").await?.body;
    let flagged = setup.get("/api/page?flagged=no&start=many&q=+").await?.body;
    let unclear = setup.get("/api/page?flagged=maybe&start=2").await?.body;

    assert_eq!(searched["total"], 2);
    assert_eq!(
        (&flagged["total"], &flagged["start"]),
        (&json!(3), &json!(0))
    );
    assert_eq!(
        (&unclear["total"], &unclear["start"]),
        (&json!(3), &json!(2))
    );

    Ok(())
}

#[tokio::test]
async fn a_record_comes_whole_and_unknown_positions_are_not_found() -> Result<()> {
    let setup = Setup::new()?;
    let record = setup.get("/api/record?position=2").await?.body;
    let unknown = setup.get("/api/record?position=99").await?;
    let unnamed = setup.get("/api/record").await?;

    assert_eq!(record["response"], "line one\nline two\nline three");
    assert_eq!(
        record["suggestions"],
        json!({ "slop": [], "violations": [] })
    );
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.body, json!({ "error": "not found: record 99" }));
    assert_eq!(unnamed.status, StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn sets_and_clears_a_flag() -> Result<()> {
    let setup = Setup::new()?;
    let body = r#"{"id":"a1","note":"flattery","spans":[{"field":"response","text":"Great question!","category":"flattery_filler_opener"}]}"#;
    let set = setup.post("/api/flag", body).await?.body;

    assert_eq!(set, json!({ "ok": true, "flagged": true }));
    assert_eq!(
        setup.app()?.flags()?.get("a1").map(|flag| flag.spans.len()),
        Some(1)
    );

    setup
        .post("/api/flag", r#"{"id":"a1","flagged":false}"#)
        .await?;

    assert_eq!(setup.app()?.flags()?.get("a1"), None);

    Ok(())
}

#[tokio::test]
async fn an_unknown_category_is_a_bad_request() -> Result<()> {
    let setup = Setup::new()?;
    let body = r#"{"id":"a1","spans":[{"field":"response","text":"x","category":"made_up"}]}"#;
    let reply = setup.post("/api/flag", body).await?;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(
        reply.body["error"]
            .as_str()
            .is_some_and(|error| error.starts_with("bad request: flag body: "))
    );

    Ok(())
}

#[tokio::test]
async fn marks_a_phrase_everywhere_as_written() -> Result<()> {
    let setup = Setup::new()?;
    let body = r#"{"text":"here's the thing","category":"dramatic_setup"}"#;
    let first = setup.post("/api/flag-matches", body).await?.body;
    let again = setup.post("/api/flag-matches", body).await?.body;

    assert_eq!(first, json!({ "matched": 2, "added": 2 }));
    assert_eq!(again, json!({ "matched": 2, "added": 0 }));

    let marked = setup
        .app()?
        .flags()?
        .get("b2")
        .map(|flag| flag.spans[0].text.clone());

    assert_eq!(marked.as_deref(), Some("Here's The Thing"));

    let phrases = setup.get("/api/phrases").await?.body;

    assert_eq!(phrases["phrases"][0]["examples"], 2);

    Ok(())
}

#[tokio::test]
async fn a_phrase_found_outside_the_answers_marks_nothing() -> Result<()> {
    let setup = Setup::new()?;
    let body = r#"{"text":"trpl/src","category":"other"}"#;
    let reply = setup.post("/api/flag-matches", body).await?.body;

    assert_eq!(reply, json!({ "matched": 0, "added": 0 }));

    Ok(())
}

#[tokio::test]
async fn refuses_a_phrase_that_is_too_short() -> Result<()> {
    let setup = Setup::new()?;
    let body = r#"{"text":" ok ","category":"other"}"#;
    let reply = setup.post("/api/flag-matches", body).await?;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body,
        json!({ "error": "bad request: a phrase needs at least 4 characters" })
    );

    Ok(())
}

#[test]
fn shortens_at_a_line_end_counting_characters() {
    assert_eq!(shorten_at_line("fn a() {}\nfn b() {}\n", 14), "fn a() {}");
    assert_eq!(shorten_at_line("abcdefgh", 3), "abc");
    assert_eq!(shorten_at_line("é\néé", 3), "é");
}

#[tokio::test]
async fn a_long_answer_is_sent_shortened_on_a_page() -> Result<()> {
    let long = "x".repeat(MAX_INLINE_RESPONSE + 10);
    let line =
        json!({ "id": "l", "source": "chat", "origin": "c", "response": format!("short\n{long}") });
    let setup = Setup::with(&format!("{line}\n"))?;
    let item = &setup.get("/api/page").await?.body["items"][0];

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

#[tokio::test]
async fn answers_over_a_real_connection() -> Result<()> {
    let setup = Setup::new()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let stop = Arc::new(Notify::new());
    let stopped = Arc::clone(&stop);
    let serving = tokio::spawn(serve(listener, Arc::clone(&setup.workspace), async move {
        stopped.notified().await
    }));
    let mut stream = TcpStream::connect(address).await?;
    let mut reply = String::new();

    stream
        .write_all(b"GET /api/datasets HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await?;
    stream.read_to_string(&mut reply).await?;
    stop.notify_one();
    serving.await??;

    assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
    assert!(reply.contains(r#""name":"train""#), "{reply}");

    Ok(())
}
