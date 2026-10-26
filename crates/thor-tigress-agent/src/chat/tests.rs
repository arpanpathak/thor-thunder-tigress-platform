use super::{
    wire::{CallPiece, Chunk, SearchPlan},
    *,
};
use crate::web::{research::MAX_QUERIES_PER_CALL, search::TimeRange};
use crate::testing::{FakeServer, FakeWeb, event_stream, json_response};

fn upstreams(model: &FakeServer, search: &FakeServer) -> Upstreams {
    upstreams_with(model, search, FakeWeb::default())
}

fn upstreams_with(model: &FakeServer, search: &FakeServer, web: FakeWeb) -> Upstreams {
    Upstreams {
        default: Engine::edge("nemotron".to_string(), model.address()),
        others: Vec::new(),
        search: Endpoint::new(search.address(), None),
        web: Box::new(web),
    }
}

/// A model reply that says `text` and stops.
fn says(text: &str) -> String {
    let chunk = json!({ "choices": [{ "delta": { "content": text } }] }).to_string();

    event_stream(&[&chunk])
}

/// A model reply that calls `tool` with `arguments`.
fn calls(tool: &str, arguments: &str) -> String {
    event_stream(&[&call_event(tool, arguments)])
}

/// A chat with web search on, once it has been answered: the events the
/// client was sent, and the two servers that were asked.
struct Searched {
    sent: Vec<String>,
    model: FakeServer,
    search: FakeServer,
}

/// Answers `messages` with web search on, against a model server and a
/// SearXNG that give the canned `replies` and `results`.
fn searched(
    messages: Value,
    replies: Vec<String>,
    results: Vec<String>,
    web: FakeWeb,
) -> Outcome<Searched> {
    let model = FakeServer::start(replies)?;
    let search = FakeServer::start(results)?;
    let request = json!({ "messages": messages, "thor_web_search": true });
    let mut client = Vec::new();

    answer(
        &mut client,
        &serde_json::to_vec(&request)?,
        &upstreams_with(&model, &search, web),
    )?;

    Ok(Searched {
        sent: events(&client),
        model,
        search,
    })
}

/// One user message asking `question`.
fn asking(question: &str) -> Value {
    json!([{ "role": "user", "content": question }])
}

fn idle() -> Outcome<FakeServer> {
    FakeServer::start(Vec::new())
}

fn events(client: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(client)
        .lines()
        .filter_map(|line| line.strip_prefix("data: ").map(ToString::to_string))
        .collect()
}

fn chunk(json: &str) -> Outcome<Chunk> {
    Ok(serde_json::from_str(json)?)
}

#[test]
fn conversation_helpers_leave_a_request_without_messages_alone() {
    let mut fields = Fields::new();
    fields.insert(MESSAGES.to_string(), json!("not a list"));
    append_system(&mut fields, "hint");
    append_user(&mut fields, "ask");
    retract_system(&mut fields, "hint");
    set_system(&mut fields, "hello".to_string());

    assert_eq!(fields[MESSAGES], json!("not a list"));

    let mut plain = Fields::new();

    plain.insert(
        MESSAGES.to_string(),
        json!([{ "role": "user", "content": "hi" }]),
    );
    retract_system(&mut plain, "hint");

    assert_eq!(plain[MESSAGES][0]["content"], "hi");
}

#[test]
fn a_call_piece_without_a_function_keeps_what_it_has() {
    let mut call = ToolCall::default();

    call.extend(CallPiece {
        index: 0,
        id: Some("call-1".to_string()),
        function: None,
    });

    assert_eq!(call.id, "call-1");
    assert!(call.name.is_empty() && call.arguments.is_empty());
}

#[test]
fn more_queries_than_the_cap_are_dropped() {
    let call = ToolCall {
        id: "1".to_string(),
        name: "web_search".to_string(),
        arguments: r#"{"query":"rust","queries":["tokio","async","jobs","threads"]}"#.to_string(),
    };
    let planned = call.search_plan().map(|plan| plan.queries.len());

    assert_eq!(planned, Some(MAX_QUERIES_PER_CALL));
}

fn call_event(name: &str, arguments: &str) -> String {
    call_event_at(0, name, arguments)
}

fn call_event_at(index: usize, name: &str, arguments: &str) -> String {
    json!({"choices":[{"delta":{"tool_calls":[{"index":index,"id":format!("c{index}"),"function":{"name":name,"arguments":arguments}}]}}]})
        .to_string()
}

#[test]
fn the_mode_is_a_truth_table() {
    assert_eq!(Mode::of(true, true, false), Mode::Search);
    assert_eq!(Mode::of(true, false, true), Mode::Search);
    assert_eq!(Mode::of(false, true, false), Mode::Stream);
    assert_eq!(Mode::of(false, true, true), Mode::Relay);
    assert_eq!(Mode::of(false, false, false), Mode::Relay);
    assert_eq!(Mode::of(false, false, true), Mode::Relay);
}

#[test]
fn tools_are_found_by_name() {
    assert_eq!(Tool::named("web_search"), Some(Tool::WebSearch));
    assert_eq!(
        Tool::named("fetch_page_content_recursive"),
        Some(Tool::FetchPage)
    );
    assert_eq!(Tool::named("fetch_page"), None);
    assert_eq!(Tool::ALL.len(), 2);
    assert_eq!(
        Tool::WebSearch.definition()["function"]["name"],
        "web_search"
    );
    assert_eq!(
        Tool::FetchPage.definition()["function"]["name"],
        "fetch_page_content_recursive"
    );
    assert_eq!(
        Tool::WebSearch.definition()["function"]["parameters"]["properties"]["time_range"]["enum"]
            [0],
        "day"
    );
}

#[test]
fn a_search_request_gains_a_system_hint() {
    let mut fields = Fields::new();

    fields.insert(
        MESSAGES.to_string(),
        json!([{ "role": "user", "content": "news?" }]),
    );
    nudge_to_search(&mut fields);

    assert_eq!(
        fields[MESSAGES],
        json!([{ "role": "system", "content": SEARCH_HINT }, { "role": "user", "content": "news?" }])
    );
}

#[test]
fn the_hint_merges_into_an_existing_system_message() {
    let mut fields = Fields::new();

    fields.insert(
        MESSAGES.to_string(),
        json!([{ "role": "system", "content": "Be brief." }, { "role": "user", "content": "hi" }]),
    );
    nudge_to_search(&mut fields);

    assert_eq!(
        fields[MESSAGES][0]["content"],
        json!(format!("Be brief.\n\n{SEARCH_HINT}"))
    );
    assert_eq!(fields[MESSAGES].as_array().map(Vec::len), Some(2));
}

#[test]
fn the_hint_is_skipped_without_a_message_list() {
    let mut fields = Fields::new();
    fields.insert(STREAM.to_string(), Value::Bool(true));
    nudge_to_search(&mut fields);

    assert!(!fields.contains_key(MESSAGES));
}

#[test]
fn taking_the_hint_back_leaves_the_rest_of_the_system_line() {
    let content = format!("Be brief.\n\n{SEARCH_HINT}\n\nBe honest.");
    let system = |fields: &Fields| fields[MESSAGES][0]["content"].clone();
    let mut fields = Fields::new();

    fields.insert(
        MESSAGES.to_string(),
        json!([{ "role": "system", "content": content }, { "role": "user", "content": "hi" }]),
    );
    retract_system(&mut fields, SEARCH_HINT);

    assert_eq!(system(&fields), json!("Be brief.\n\nBe honest."));
    assert_eq!(fields[MESSAGES].as_array().map(Vec::len), Some(2));
}

#[test]
fn taking_back_a_hint_that_is_the_whole_system_line_leaves_it_empty() {
    let mut fields = Fields::new();

    fields.insert(
        MESSAGES.to_string(),
        json!([{ "role": "system", "content": SEARCH_HINT }]),
    );
    retract_system(&mut fields, SEARCH_HINT);

    assert_eq!(fields[MESSAGES][0]["content"], json!(""));
}

#[test]
fn taking_back_a_hint_that_was_never_added_changes_nothing() {
    let messages = json!([{ "role": "system", "content": "Be brief." }]);
    let mut fields = Fields::new();
    fields.insert(MESSAGES.to_string(), messages.clone());
    retract_system(&mut fields, SEARCH_HINT);

    assert_eq!(fields[MESSAGES], messages);
}

#[test]
fn a_system_message_that_is_not_text_is_left_alone() {
    let messages = json!([{ "role": "system", "content": [{ "type": "text", "text": "hi" }] }]);
    let mut fields = Fields::new();
    fields.insert(MESSAGES.to_string(), messages.clone());
    nudge_to_search(&mut fields);

    assert_eq!(fields[MESSAGES], messages);
}

#[test]
fn assembles_a_tool_call_streamed_in_pieces() -> Outcome {
    let mut round = Round::default();
    round.absorb(chunk(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a1","function":{"name":"web_search","arguments":"{\"que"}}]}}]}"#)?);
    round.absorb(chunk(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ry\":\"rust\"}"}}]}}]}"#)?);
    round.absorb(chunk(
        r#"{"choices":[{"delta":{"content":"ok","tool_calls":null}}]}"#,
    )?);
    round.absorb(chunk(r#"{"choices":[],"timings":{}}"#)?);

    assert_eq!(round.content, "ok");
    assert_eq!(round.calls.len(), 1);
    assert_eq!(
        round.calls[0].search_plan(),
        Some(SearchPlan {
            queries: vec!["rust".to_string()],
            range: None
        })
    );

    Ok(())
}

#[test]
fn ignores_tool_calls_past_the_limit() -> Outcome {
    let mut round = Round::default();

    round.absorb(chunk(
        r#"{"choices":[{"delta":{"tool_calls":[{"index":1000000,"id":"x"}]}}]}"#,
    )?);

    assert_eq!(round.calls, []);

    Ok(())
}

/// The plan of a `web_search` call with `arguments`.
fn plan(arguments: &str) -> Option<SearchPlan> {
    ToolCall {
        arguments: arguments.to_string(),
        ..ToolCall::default()
    }
    .search_plan()
}

/// The queries of a `web_search` call with `arguments`.
fn planned(arguments: &str) -> Option<Vec<String>> {
    plan(arguments).map(|plan| plan.queries)
}

fn owned(queries: &[&str]) -> Option<Vec<String>> {
    Some(queries.iter().map(ToString::to_string).collect())
}

#[test]
fn a_search_plan_holds_each_query_once_and_trimmed() {
    let several = r#"{"query":"rust","queries":["rust","tokio","  ","axum"]}"#;

    assert_eq!(planned(r#"{"query":"  rust  "}"#), owned(&["rust"]));
    assert_eq!(planned(several), owned(&["rust", "tokio", "axum"]));
    assert_eq!(
        planned(r#"{"queries":["only this"]}"#),
        owned(&["only this"])
    );
    assert_eq!(plan(r#"{"query":"  "}"#), None);
    assert_eq!(plan(r#"{"q":"rust"}"#), None);
    assert_eq!(plan("not json"), None);
}

#[test]
fn a_search_plan_reads_a_known_recency_only() {
    let range = |arguments: &str| plan(arguments).and_then(|plan| plan.range);

    assert_eq!(
        range(r#"{"query":"jobs","time_range":"week"}"#),
        Some(TimeRange::Week)
    );
    assert_eq!(range(r#"{"query":"jobs","time_range":"forever"}"#), None);
    assert_eq!(range(r#"{"query":"jobs"}"#), None);
}

#[test]
fn a_search_plan_is_widened_by_its_kind_and_capped() {
    let jobs = owned(&[
        "rust engineer",
        "rust engineer job posting",
        "rust engineer careers",
        "rust engineer linkedin jobs",
    ]);
    let people = planned(r#"{"query":"synthires","kind":"people"}"#);
    let two_people = planned(r#"{"queries":["a","b"],"kind":"people"}"#);

    assert_eq!(planned(r#"{"query":"rust engineer","kind":"jobs"}"#), jobs);
    assert_eq!(
        people.map(|queries| queries.len()),
        Some(MAX_QUERIES_PER_CALL)
    );
    assert_eq!(
        two_people.map(|queries| queries.len()),
        Some(MAX_QUERIES_PER_CALL)
    );
}

#[test]
fn a_tool_call_must_carry_its_argument() {
    let call = |arguments: &str| ToolCall {
        arguments: arguments.to_string(),
        ..ToolCall::default()
    };

    assert_eq!(
        call(r#"{"url":" https://example.com/ "}"#)
            .url_argument()
            .as_deref(),
        Some("https://example.com/")
    );
    assert_eq!(call(r#"{"url":"  "}"#).url_argument(), None);
    assert_eq!(call("[]").url_argument(), None);
}

#[test]
fn a_search_call_with_several_queries_fans_out() -> Outcome {
    let call = r#"{"query":"rust jobs","queries":["tokio jobs"]}"#;
    let results = vec![
        json_response(
            r#"{"results":[{"title":"A","url":"https://a"},{"title":"B","url":"https://b"}]}"#,
        ),
        json_response(
            r#"{"results":[{"title":"B again","url":"https://b"},{"title":"C","url":"https://c"}]}"#,
        ),
    ];
    let replies = vec![calls("web_search", call), says("done")];
    let chat = searched(asking("jobs"), replies, results, FakeWeb::default())?;
    let asked = chat.search.requests()?;
    let seen = chat.model.requests()?;
    let searches = chat
        .sent
        .iter()
        .filter(|event| event.contains(r#""search""#));

    assert_eq!(asked.len(), 2);
    assert!(asked[0].contains("q=rust+jobs"));
    assert!(asked[1].contains("q=tokio+jobs"));
    assert!(seen[1].contains("[1] A"), "{}", seen[1]);
    assert!(seen[1].contains("[2] B"), "{}", seen[1]);
    assert!(seen[1].contains("[3] C"), "{}", seen[1]);
    assert!(seen[1].contains("Searches used 2 of 16"), "{}", seen[1]);
    assert_eq!(searches.count(), 2);

    Ok(())
}

#[test]
fn a_people_search_is_widened_towards_recruiters() -> Outcome {
    let call = call_event(
        "web_search",
        r#"{"query":"synthires rust","kind":"people"}"#,
    );
    let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
    let search = FakeServer::start(vec![
        json_response(r#"{"results":[]}"#);
        MAX_QUERIES_PER_CALL
    ])?;

    answer(
        &mut Vec::new(),
        br#"{"messages":[{"role":"user","content":"who hires"}],"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;

    let asked = search.requests()?;

    assert_eq!(asked.len(), MAX_QUERIES_PER_CALL);
    assert!(asked[0].contains("q=synthires+rust&"), "{}", asked[0]);
    assert!(
        asked[1].contains("q=synthires+rust+recruiter"),
        "{}",
        asked[1]
    );
    assert!(asked[2].contains("hiring+manager"), "{}", asked[2]);
    assert!(asked[3].contains("we+are+hiring"), "{}", asked[3]);

    Ok(())
}

#[test]
fn the_search_budget_stops_the_seventeenth_query() -> Outcome {
    let wide = |call: usize| {
        call_event_at(
            call,
            "web_search",
            &format!(r#"{{"queries":["q{call}a","q{call}b","q{call}c","q{call}d"]}}"#),
        )
    };
    let stream = event_stream(&[&wide(0), &wide(1), &wide(2), &wide(3), &wide(4)]);
    let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
    let model = FakeServer::start(vec![stream, event_stream(&[reply])])?;
    let search = FakeServer::start(vec![json_response(r#"{"results":[]}"#); MAX_SEARCHES])?;

    answer(
        &mut Vec::new(),
        br#"{"messages":[{"role":"user","content":"hunt"}],"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;

    assert_eq!(search.requests()?.len(), MAX_SEARCHES);

    let seen = model.requests()?;

    assert_eq!(seen.len(), 2);
    assert!(seen[1].contains("budget is spent"), "{}", seen[1]);
    assert!(seen[1].contains("Searches used 16 of 16"), "{}", seen[1]);

    Ok(())
}

#[test]
fn the_answer_round_is_given_the_source_numbers() -> Outcome {
    let mut replies: Vec<String> = (1..=MAX_ROUNDS)
        .map(|round| calls("web_search", &format!(r#"{{"query":"q{round}"}}"#)))
        .collect();

    replies.push(says("done"));

    let found = json_response(r#"{"results":[{"title":"Rust","url":"https://r"}]}"#);
    let results = vec![found; MAX_ROUNDS];
    let chat = searched(asking("news"), replies, results, FakeWeb::default())?;
    let seen = chat.model.requests()?;
    let last = &seen[MAX_ROUNDS];

    assert_eq!(seen.len(), MAX_ROUNDS + 1);
    assert!(
        last.contains("Sources found, with the numbers to cite"),
        "{last}"
    );
    assert!(last.contains("[1] Rust — https://r"), "{last}");
    assert!(
        !last.contains(SEARCH_HINT),
        "the search hint must be gone by the answer rounds"
    );

    Ok(())
}

#[test]
fn added_messages_have_openais_shape() -> Outcome {
    let round = Round {
        content: "thinking".to_string(),
        calls: vec![ToolCall {
            id: "c1".to_string(),
            name: "web_search".to_string(),
            arguments: "{}".to_string(),
        }],
        cut: false,
    };

    assert_eq!(
        serde_json::to_value(round.as_message())?,
        json!({"role":"assistant","content":"thinking","tool_calls":[{"id":"c1","type":"function","function":{"name":"web_search","arguments":"{}"}}]})
    );
    assert_eq!(
        serde_json::to_value(Added::Tool {
            tool_call_id: "c1",
            content: "[1] x"
        })?,
        json!({"role":"tool","tool_call_id":"c1","content":"[1] x"})
    );

    Ok(())
}

#[test]
fn relays_a_request_that_does_not_stream() -> Outcome {
    let (model, search) = (
        FakeServer::start(vec![json_response(r#"{"choices":[]}"#)])?,
        idle()?,
    );
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"messages":[]}"#,
        &upstreams(&model, &search),
    )?;

    assert!(String::from_utf8_lossy(&client).ends_with(r#"{"choices":[]}"#));
    assert!(model.requests()?[0].ends_with(r#"{"messages":[],"model":"nemotron"}"#));

    Ok(())
}

#[test]
fn streams_tokens_and_ends_with_done() -> Outcome {
    let token = r#"{"choices":[{"delta":{"content":"hi"}}]}"#;
    let (model, search) = (FakeServer::start(vec![event_stream(&[token])])?, idle()?);
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"messages":[],"stream":true}"#,
        &upstreams(&model, &search),
    )?;

    assert_eq!(events(&client), [token, DONE]);
    assert!(!model.requests()?[0].contains(SEARCH_HINT));

    Ok(())
}

/// A chunk whose text is all tool-call tag still carries its thinking, so it
/// is forwarded with the text emptied; one that carries nothing else is dropped.
#[test]
fn a_chunk_that_is_only_a_tag_is_forwarded_for_what_else_it_carries() -> Outcome {
    let thinking = r#"{"choices":[{"delta":{"content":"<tool_call>","reasoning_content":"hm"}}]}"#;
    let bare = r#"{"choices":[{"delta":{"content":"</tool_call>"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[thinking, bare])])?;
    let search = idle()?;
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"messages":[],"stream":true}"#,
        &upstreams(&model, &search),
    )?;

    let sent = events(&client);

    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent[0].contains(r#""reasoning_content":"hm""#), "{sent:?}");
    assert!(sent[0].contains(r#""content":"""#), "{sent:?}");

    Ok(())
}

#[test]
fn a_web_search_request_tells_the_model_to_search() -> Outcome {
    let call = call_event("web_search", r#"{"query":"rust"}"#);
    let reply = r#"{"choices":[{"delta":{"content":"ok"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
    let search = FakeServer::start(vec![json_response(r#"{"results":[]}"#)])?;
    let request = br#"{"messages":[{"role":"user","content":"news?"}],"thor_web_search":true}"#;
    answer(&mut Vec::new(), request, &upstreams(&model, &search))?;

    let seen = model.requests()?;

    assert!(seen[0].contains(SEARCH_HINT) && seen[0].contains(r#""role":"system""#));

    Ok(())
}

#[test]
fn searches_then_answers() -> Outcome {
    let call = call_event("web_search", r#"{"query":"rust"}"#);
    let reply = r#"{"choices":[{"delta":{"content":"Rust 1.99"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
    let search = FakeServer::start(vec![json_response(
        r#"{"results":[{"title":"Rust","url":"https://r","content":"new"}]}"#,
    )])?;
    let mut client = Vec::new();
    let request = br#"{"messages":[{"role":"user","content":"news?"}],"thor_web_search":true}"#;
    answer(&mut client, request, &upstreams(&model, &search))?;

    let seen = model.requests()?;

    assert!(seen[0].contains(r#""tools""#) && seen[0].contains(r#""stream":true"#));
    assert!(seen[1].contains(r#""role":"tool""#) && seen[1].contains("[1] Rust"));

    let sent = events(&client);

    assert_eq!(sent.first(), Some(&call));
    assert!(sent.contains(&r#"{"thor":{"search":{"query":"rust","results":[{"domain":"r","title":"Rust","url":"https://r"}]}}}"#.to_string()), "{sent:?}");
    assert_eq!(sent.last().map(String::as_str), Some(DONE));

    Ok(())
}

#[test]
fn every_tool_round_has_tools_and_one_last_round_does_not() -> Outcome {
    let call = call_event("other", "{}");
    let reply = r#"{"choices":[{"delta":{"content":"final answer"}}]}"#;
    let mut responses = vec![event_stream(&[&call]); MAX_ROUNDS];
    responses.push(event_stream(&[reply]));

    let model = FakeServer::start(responses)?;

    answer(
        &mut Vec::new(),
        br#"{"messages":[],"thor_web_search":true}"#,
        &upstreams(&model, &idle()?),
    )?;

    let seen = model.requests()?;

    assert_eq!(seen.len(), MAX_ROUNDS + 1);
    assert!(
        seen[..MAX_ROUNDS]
            .iter()
            .all(|request| request.contains(r#""tools""#))
    );
    assert!(!seen[MAX_ROUNDS].contains(r#""tools""#));
    assert!(seen[MAX_ROUNDS].contains(ANSWER_NUDGE));
    assert!(seen[MAX_ROUNDS].contains(ANSWER_ASK));
    assert!(seen[1].contains("Unknown tool other."));
    assert!(seen[2].contains("already ran"), "{}", seen[2]);

    Ok(())
}

#[test]
fn a_call_written_as_text_runs_and_is_not_shown() -> Outcome {
    let text = "Let me look.\n<tool_call>\n<function=web_search>\n<parameter=query>\nrust jobs\n</parameter>\n</function>\n</tool_call>\nDone.";
    let replies = vec![says(text), says("Here is the answer")];
    let results = vec![json_response(
        r#"{"results":[{"title":"A job","url":"https://jobs.example/1","content":"hiring"}]}"#,
    )];
    let chat = searched(asking("jobs?"), replies, results, FakeWeb::default())?;
    let seen = chat.model.requests()?;
    let shown = |text: &str| chat.sent.iter().any(|event| event.contains(text));

    assert!(seen[1].contains("[1] A job"), "{}", seen[1]);
    assert!(chat.search.requests()?[0].contains("q=rust+jobs"));
    assert!(!shown("<tool_call"), "{:?}", chat.sent);
    assert!(shown("Let me look."), "{:?}", chat.sent);
    assert!(shown("Here is the answer"), "{:?}", chat.sent);

    Ok(())
}

#[test]
fn a_tag_the_model_leaves_open_is_flushed_at_the_end_of_the_round() -> Outcome {
    let call = call_event("web_search", r#"{"query":"rust"}"#);
    let partial = json!({ "choices": [{ "delta": { "content": "almost <tool" } }] }).to_string();
    let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[&partial])])?;
    let search = FakeServer::start(vec![json_response(r#"{"results":[]}"#)])?;
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"messages":[{"role":"user","content":"news"}],"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;

    let sent = events(&client);

    assert!(
        sent.iter().any(|event| event.contains("almost ")),
        "{sent:?}"
    );
    assert!(sent.iter().any(|event| event.contains("<tool")), "{sent:?}");

    Ok(())
}

#[test]
fn a_query_that_fails_is_reported_and_the_others_still_run() -> Outcome {
    let call = call_event("web_search", r#"{"query":"good","queries":["bad"]}"#);
    let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
    let search = FakeServer::start(vec![json_response(
        r#"{"results":[{"title":"Good","url":"https://good"}]}"#,
    )])?;

    answer(
        &mut Vec::new(),
        br#"{"messages":[{"role":"user","content":"news"}],"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;

    let seen = model.requests()?;

    assert!(seen[1].contains("bad"), "{}", seen[1]);
    assert!(seen[1].contains("failed"), "{}", seen[1]);
    assert!(seen[1].contains("[1] Good"), "{}", seen[1]);
    assert!(seen[1].contains("Searches used 2 of 16"), "{}", seen[1]);

    Ok(())
}

#[test]
fn engines_that_did_not_answer_reach_the_model() -> Outcome {
    let call = call_event("web_search", r#"{"query":"rust"}"#);
    let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
    let search = FakeServer::start(vec![json_response(
        r#"{"results":[],"unresponsive_engines":[["duckduckgo","CAPTCHA"],["brave",null]]}"#,
    )])?;

    answer(
        &mut Vec::new(),
        br#"{"messages":[{"role":"user","content":"news"}],"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;

    let seen = model.requests()?;

    assert!(
        seen[1].contains("Engines that did not answer: duckduckgo (CAPTCHA), brave."),
        "{}",
        seen[1]
    );

    Ok(())
}

/// Six pages on one site, each linking to all six.
fn linked_pages() -> FakeWeb {
    let names = ["a", "b", "c", "d", "e", "f"];
    let links: String = names
        .iter()
        .map(|name| format!("<a href=\"/{name}\">{name}</a>"))
        .collect();
    let urls = names.map(|name| format!("https://docs.example/{name}"));
    let pages = names
        .iter()
        .zip(&urls)
        .map(|(name, url)| {
            let page = format!("<title>{}</title>{links}", name.to_uppercase());

            (url.as_str(), fetched("text/html", &page))
        })
        .collect();

    FakeWeb::new(pages)
}

#[test]
fn the_page_budget_stops_after_twelve_pages() -> Outcome {
    let fetch = |page: &str| {
        let arguments = format!(r#"{{"url":"https://docs.example/{page}"}}"#);

        calls("fetch_page_content_recursive", &arguments)
    };
    let replies = vec![fetch("a"), fetch("b"), fetch("c"), says("done")];
    let question = "read https://docs.example/a https://docs.example/b https://docs.example/c";
    let chat = searched(asking(question), replies, Vec::new(), linked_pages())?;
    let seen = chat.model.requests()?;

    assert!(seen[3].contains("page budget is spent"), "{}", seen[3]);
    assert!(seen[3].contains("(12 pages)"), "{}", seen[3]);

    Ok(())
}

#[test]
fn a_call_on_the_answer_round_is_run() -> Outcome {
    let leaked =
        "<tool_call><function=web_search><parameter=query>x</parameter></function></tool_call>";
    let mut replies = vec![calls("other", "{}"); MAX_ROUNDS];

    replies.push(says(leaked));
    replies.push(says("the answer"));

    let results = vec![json_response(r#"{"results":[]}"#)];
    let chat = searched(json!([]), replies, results, FakeWeb::default())?;
    let seen = chat.model.requests()?;
    let last = &seen[MAX_ROUNDS + 1];
    let shown = |text: &str| chat.sent.iter().any(|event| event.contains(text));

    assert_eq!(seen.len(), MAX_ROUNDS + 2);
    assert!(last.contains("No results."), "{last}");
    assert!(chat.search.requests()?[0].contains("q=x"));
    assert!(!shown("<tool_call"), "{:?}", chat.sent);
    assert!(shown("the answer"), "{:?}", chat.sent);

    Ok(())
}

/// A reply that writes `text` and then calls `tool` with `arguments`.
fn says_and_calls(text: &str, tool: &str, arguments: &str) -> String {
    let chunk = json!({ "choices": [{ "delta": { "content": text } }] }).to_string();

    event_stream(&[&chunk, &call_event(tool, arguments)])
}

/// How many of the events sent to the page carry `text`.
fn times_shown(sent: &[String], text: &str) -> usize {
    sent.iter().filter(|event| event.contains(text)).count()
}

// Each fake server takes exactly the replies scripted, so a round past the
// answer finds no model and the page is sent an error.
#[test]
fn an_answer_followed_by_a_search_already_run_is_written_once() -> Outcome {
    let search = r#"{"query":"rust"}"#;
    let answer = "Fit assessment: a strong match.";
    let replies = vec![
        calls("web_search", search),
        says_and_calls(answer, "web_search", search),
    ];

    let results = vec![json_response(r#"{"results":[]}"#)];
    let chat = searched(asking("fit?"), replies, results, FakeWeb::default())?;

    assert_eq!(
        times_shown(&chat.sent, r#"{"thor":{"error""#),
        0,
        "{:?}",
        chat.sent
    );
    assert_eq!(chat.model.requests()?.len(), 2);

    Ok(())
}

#[test]
fn a_long_answer_followed_by_a_new_search_is_written_once() -> Outcome {
    let answer = (1..=60)
        .map(|n| format!("Requirement {n} is met. "))
        .collect::<String>();
    let replies = vec![
        calls("web_search", r#"{"query":"rust"}"#),
        says_and_calls(&answer, "web_search", r#"{"query":"rust jobs"}"#),
    ];

    let results = vec![json_response(r#"{"results":[]}"#)];
    let chat = searched(asking("fit?"), replies, results, FakeWeb::default())?;

    assert_eq!(
        times_shown(&chat.sent, r#"{"thor":{"error""#),
        0,
        "{:?}",
        chat.sent
    );
    assert_eq!(chat.model.requests()?.len(), 2);
    assert_eq!(chat.search.requests()?.len(), 1);

    Ok(())
}

#[test]
fn text_beside_a_call_on_the_answer_round_does_not_end_the_answer() -> Outcome {
    let call = call_event("other", "{}");
    let mixed = json!({ "choices": [{ "delta": { "content": "Here are the jobs. <tool_call><function=web_search><parameter=query>x</parameter></function></tool_call>" } }] }).to_string();
    let reply = r#"{"choices":[{"delta":{"content":"and the links: https://jobs.example/1"}}]}"#;
    let mut responses = vec![event_stream(&[&call]); MAX_ROUNDS];
    responses.push(event_stream(&[&mixed]));
    responses.push(event_stream(&[reply]));

    let model = FakeServer::start(responses)?;
    let search = FakeServer::start(vec![json_response(r#"{"results":[]}"#)])?;
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"messages":[],"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;

    let seen = model.requests()?;

    assert_eq!(seen.len(), MAX_ROUNDS + 2);
    assert!(search.requests()?[0].contains("q=x"));

    let sent = events(&client);

    assert!(
        sent.iter()
            .any(|event| event.contains("Here are the jobs.")),
        "{sent:?}"
    );
    assert!(
        sent.iter()
            .any(|event| event.contains("and the links: https://jobs.example/1")),
        "{sent:?}"
    );

    Ok(())
}

#[test]
fn the_fallback_is_sent_when_no_round_writes_an_answer() -> Outcome {
    let call = call_event("other", "{}");
    let empty = r#"{"choices":[{"delta":{}}]}"#;
    let mut responses = vec![event_stream(&[&call]); MAX_ROUNDS];
    responses.extend((0..ANSWER_ROUNDS).map(|_| event_stream(&[empty])));

    let model = FakeServer::start(responses)?;
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"messages":[],"thor_web_search":true}"#,
        &upstreams(&model, &idle()?),
    )?;

    let sent = events(&client);

    assert!(
        sent.iter()
            .any(|event| event.contains("no round wrote an answer")),
        "{sent:?}"
    );

    Ok(())
}

#[test]
fn the_fallback_names_the_sources_when_no_round_writes_one() -> Outcome {
    let call = call_event("web_search", r#"{"query":"nvidia jobs"}"#);
    let empty = r#"{"choices":[{"delta":{}}]}"#;
    let mut responses = vec![event_stream(&[&call]); MAX_ROUNDS];
    responses.extend((0..ANSWER_ROUNDS).map(|_| event_stream(&[empty])));

    let model = FakeServer::start(responses)?;
    let search = FakeServer::start(vec![json_response(
        r#"{"results":[{"title":"A job","url":"https://jobs.example/1"}]}"#,
    )])?;
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"messages":[],"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;

    let sent = events(&client);

    assert!(
        sent.iter()
            .any(|event| event.contains("[1] A job — https://jobs.example/1")),
        "{sent:?}"
    );

    Ok(())
}

#[test]
fn a_model_error_becomes_an_error_event() -> Outcome {
    let (model, search) = (
        FakeServer::start(vec!["HTTP/1.1 500 Oops\r\n\r\nbroken".to_string()])?,
        idle()?,
    );
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"messages":[],"stream":true}"#,
        &upstreams(&model, &search),
    )?;

    let sent = events(&client);

    assert_eq!(
        sent,
        [
            r#"{"thor":{"error":"upstream: model server returned 500: broken"}}"#,
            DONE
        ]
    );

    Ok(())
}

#[test]
fn rejects_a_body_that_is_not_an_object() -> Outcome {
    let (model, search) = (idle()?, idle()?);
    let outcome = answer(&mut Vec::new(), b"[]", &upstreams(&model, &search));

    assert!(
        outcome
            .is_err_and(|error| error.to_string() == "bad request: the body must be a JSON object")
    );

    Ok(())
}

#[test]
fn only_the_first_tool_round_forces_a_call() {
    let llama = Engine::llama("127.0.0.1:8079".to_string(), None);

    assert_eq!(Phase::of_tool_round(1, &llama), Phase::Opening);
    assert_eq!(Phase::of_tool_round(2, &llama), Phase::Searching);
    assert_eq!(Phase::of_tool_round(MAX_ROUNDS, &llama), Phase::Searching);
}

#[test]
fn an_edge_engine_is_never_forced_to_call() {
    let edge = Engine::edge(
        "Qwen3.6-35B-A3B-NVFP4".to_string(),
        "127.0.0.1:8081".to_string(),
    );

    assert_eq!(Phase::of_tool_round(1, &edge), Phase::Searching);
    assert_eq!(Phase::of_tool_round(2, &edge), Phase::Searching);
}

#[test]
fn the_phases_offer_the_tools_and_only_the_first_forces_a_call() {
    let tools = |fields: &Fields| fields.contains_key(TOOLS);
    let forced = |fields: &Fields| fields.contains_key(TOOL_CHOICE);

    let mut fields = Fields::new();
    prepare(&mut fields, Phase::Opening);

    assert!(tools(&fields));
    assert!(forced(&fields));
    assert_eq!(fields[TOOL_CHOICE], json!("required"));

    prepare(&mut fields, Phase::Searching);

    assert!(tools(&fields));
    assert!(!forced(&fields));

    prepare(&mut fields, Phase::Answering);

    assert!(!tools(&fields));
    assert!(!forced(&fields));
}

#[test]
fn the_user_text_is_only_the_users_messages() {
    let messages = json!({
        "messages": [
            { "role": "system", "content": "hint" },
            { "role": "user", "content": "read https://user.example/doc" },
            { "role": "assistant", "content": "sure" },
            { "role": "user", "content": [{ "type": "text" }] }
        ]
    });
    let fields = messages.as_object().cloned().unwrap_or_default();

    assert_eq!(user_text(&fields), "read https://user.example/doc");
    assert_eq!(user_text(&Fields::new()), "");
}

#[test]
fn the_user_text_of_a_message_with_files_is_its_text_parts() {
    let messages = json!({
        "messages": [
            { "role": "user", "content": [
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } },
                { "type": "text", "text": "what is on https://user.example/page" }
            ] }
        ]
    });
    let fields = messages.as_object().cloned().unwrap_or_default();

    assert_eq!(user_text(&fields), "what is on https://user.example/page");
}

fn fetched(content_type: &str, body: &str) -> crate::web::http::Fetched {
    crate::web::http::Fetched {
        status: 200,
        content_type: content_type.to_string(),
        location: None,
        body: body.to_string(),
    }
}

#[test]
fn searches_then_answers_with_a_forced_first_round_and_recency() -> Outcome {
    let call = call_event("web_search", r#"{"query":"rust","time_range":"day"}"#);
    let reply = r#"{"choices":[{"delta":{"content":"Rust 1.99"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
    let search = FakeServer::start(vec![json_response(
        r#"{"results":[{"title":"Rust","url":"https://r","content":"new","publishedDate":"2026-10-06"}]}"#,
    )])?;
    let mut client = Vec::new();
    let request = br#"{"messages":[{"role":"user","content":"news?"}],"thor_web_search":true}"#;
    let mut servers = upstreams(&model, &search);
    servers.default = Engine::llama(model.address(), None);
    answer(&mut client, request, &servers)?;

    let seen = model.requests()?;

    assert!(seen[0].contains(r#""tools""#) && seen[0].contains(r#""stream":true"#));
    assert!(
        seen[0].contains(r#""tool_choice":"required""#),
        "{}",
        seen[0]
    );
    assert!(!seen[1].contains(r#""tool_choice""#));
    assert!(seen[1].contains(r#""role":"tool""#) && seen[1].contains("[1] Rust"));
    assert!(seen[1].contains("published: 2026-10-06"), "{}", seen[1]);
    assert!(search.requests()?[0].contains("time_range=day"));

    let sent = events(&client);

    assert_eq!(sent.first(), Some(&call));
    assert!(sent.contains(&r#"{"thor":{"search":{"query":"rust","results":[{"domain":"r","published":"2026-10-06","title":"Rust","url":"https://r"}]}}}"#.to_string()), "{sent:?}");
    assert_eq!(sent.last().map(String::as_str), Some(DONE));

    Ok(())
}

#[test]
fn an_edge_engine_chooses_its_first_search_itself() -> Outcome {
    let call = call_event("web_search", r#"{"query":"rust"}"#);
    let reply = r#"{"choices":[{"delta":{"content":"Rust 1.99"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
    let search = FakeServer::start(vec![json_response(
        r#"{"results":[{"title":"Rust","url":"https://r","content":"new"}]}"#,
    )])?;
    let mut client = Vec::new();
    let request = br#"{"messages":[{"role":"user","content":"news?"}],"thor_web_search":true}"#;

    answer(&mut client, request, &upstreams(&model, &search))?;

    let seen = model.requests()?;

    assert!(seen[0].contains(r#""tools""#));
    assert!(!seen[0].contains(r#""tool_choice""#), "{}", seen[0]);

    Ok(())
}

#[test]
fn reads_a_cited_page_with_the_fetch_tool() -> Outcome {
    let replies = vec![
        calls("web_search", r#"{"query":"rust"}"#),
        calls(
            "fetch_page_content_recursive",
            r#"{"url":"https://docs.example/guide"}"#,
        ),
        says("done"),
    ];
    let results = vec![json_response(
        r#"{"results":[{"title":"Guide","url":"https://docs.example/guide","content":"see"}]}"#,
    )];
    let web = FakeWeb::new(vec![(
        "https://docs.example/guide",
        fetched("text/html", "<title>Guide</title><p>the answer</p>"),
    )]);
    let read = json!({ "thor": { "read": { "url": "https://docs.example/guide", "title": "Guide", "domain": "docs.example" } } });
    let chat = searched(asking("how?"), replies, results, web)?;
    let seen = chat.model.requests()?;

    assert!(seen[2].contains("the answer"), "{}", seen[2]);
    assert!(chat.sent.contains(&read.to_string()), "{:?}", chat.sent);

    Ok(())
}

#[test]
fn an_address_the_user_wrote_may_be_read() -> Outcome {
    let fetch_call = call_event(
        "fetch_page_content_recursive",
        r#"{"url":"https://user.example/doc"}"#,
    );
    let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&fetch_call]), event_stream(&[reply])])?;
    let web = FakeWeb::new(vec![(
        "https://user.example/doc",
        fetched("text/plain", "hello from the page"),
    )]);

    answer(
        &mut Vec::new(),
        br#"{"messages":[{"role":"user","content":"read https://user.example/doc"}],"thor_web_search":true}"#,
        &upstreams_with(&model, &idle()?, web),
    )?;

    assert!(model.requests()?[1].contains("hello from the page"));

    Ok(())
}

#[test]
fn an_address_that_was_not_seen_is_refused() -> Outcome {
    let fetch_call = call_event(
        "fetch_page_content_recursive",
        r#"{"url":"https://evil.example/"}"#,
    );
    let reply = r#"{"choices":[{"delta":{"content":"ok"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&fetch_call]), event_stream(&[reply])])?;
    let web = FakeWeb::new(vec![(
        "https://evil.example/",
        fetched("text/html", "secret"),
    )]);

    answer(
        &mut Vec::new(),
        br#"{"messages":[{"role":"user","content":"hi"}],"thor_web_search":true}"#,
        &upstreams_with(&model, &idle()?, web),
    )?;

    let seen = model.requests()?;

    assert!(
        seen[1].contains("not in this answer's search results"),
        "{}",
        seen[1]
    );
    assert!(!seen[1].contains("secret"));

    Ok(())
}

#[test]
fn a_page_that_cannot_be_read_is_reported() -> Outcome {
    let fetch_call = call_event(
        "fetch_page_content_recursive",
        r#"{"url":"https://gone.example/"}"#,
    );
    let reply = r#"{"choices":[{"delta":{"content":"ok"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&fetch_call]), event_stream(&[reply])])?;

    answer(
        &mut Vec::new(),
        br#"{"messages":[{"role":"user","content":"read https://gone.example/"}],"thor_web_search":true}"#,
        &upstreams(&model, &idle()?),
    )?;

    let seen = model.requests()?;

    assert!(seen[1].contains("Could not read the page"), "{}", seen[1]);

    Ok(())
}

#[test]
fn a_fetch_without_an_address_or_with_a_bad_one_says_so() -> Outcome {
    let model = FakeServer::start(vec![
        event_stream(&[&call_event("fetch_page_content_recursive", "{}")]),
        event_stream(&[&call_event(
            "fetch_page_content_recursive",
            r#"{"url":"ftp://example.com"}"#,
        )]),
        event_stream(&[r#"{"choices":[{"delta":{"content":"ok"}}]}"#]),
    ])?;

    answer(
        &mut Vec::new(),
        br#"{"messages":[{"role":"user","content":"hi"}],"thor_web_search":true}"#,
        &upstreams(&model, &idle()?),
    )?;

    let seen = model.requests()?;

    assert!(
        seen[2].contains("The fetch needs an address."),
        "{}",
        seen[2]
    );
    assert!(
        seen[2].contains("refused: ftp://example.com: only https addresses may be read"),
        "{}",
        seen[2]
    );

    Ok(())
}

#[test]
fn a_search_without_messages_is_reported() -> Outcome {
    let (model, search) = (
        FakeServer::start(vec![event_stream(&[&call_event("web_search", "{}")])])?,
        idle()?,
    );
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;

    assert!(
        events(&client)
            .iter()
            .any(|event| event.contains("messages must be a list"))
    );

    Ok(())
}

#[test]
fn a_chunks_text_is_its_content_and_its_thinking() {
    let both = json!({ "choices": [{ "delta": { "content": "abc", "reasoning_content": "de" } }] });

    assert_eq!(text_length(&both), 5);
    assert_eq!(text_length(&json!({ "choices": [{ "delta": {} }] })), 0);
}

#[test]
fn a_round_that_runs_away_is_cut_and_the_stream_ends() -> Outcome {
    let huge = "x".repeat(MAX_ROUND_CHARS + 1);
    let chunk = json!({ "choices": [{ "delta": { "content": huge } }] }).to_string();
    let model = FakeServer::start(vec![event_stream(&[&chunk])])?;
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"messages":[{"role":"user","content":"hi"}],"stream":true}"#,
        &upstreams(&model, &idle()?),
    )?;

    let sent = events(&client);

    assert!(
        sent.iter()
            .any(|event| event.contains(r#""finish_reason":"length""#)),
        "{sent:?}"
    );
    assert_eq!(sent.last().map(String::as_str), Some(DONE));

    Ok(())
}

#[test]
fn a_round_under_the_cap_is_streamed_whole() -> Outcome {
    let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[reply])])?;
    let mut client = Vec::new();

    answer(
        &mut client,
        br#"{"messages":[],"stream":true}"#,
        &upstreams(&model, &idle()?),
    )?;

    let sent = events(&client);

    assert!(sent.iter().any(|event| event.contains("done")), "{sent:?}");
    assert!(
        !sent.iter().any(|event| event.contains("finish_reason")),
        "{sent:?}"
    );

    Ok(())
}

#[test]
fn repeated_or_blank_generation_closes_with_an_explanation() -> Outcome {
    for text in [
        ".or_else(|| DeviceBuffer::new(len))\n".repeat(12),
        " ".repeat(2048),
    ] {
        let chunk = json!({"choices":[{"delta":{"content":text}}]}).to_string();
        let after = r#"{"choices":[{"delta":{"content":"must not appear"}}]}"#;
        let model = FakeServer::start(vec![event_stream(&[&chunk, after])])?;
        let mut client = Vec::new();

        answer(
            &mut client,
            br#"{"messages":[],"stream":true}"#,
            &upstreams(&model, &idle()?),
        )?;

        let sent = events(&client);

        assert!(sent.iter().any(|event| event.contains("Stopped because")));
        assert!(!sent.iter().any(|event| event.contains("must not appear")));
        assert_eq!(sent.last().map(String::as_str), Some(DONE));
    }

    Ok(())
}

/// What a coding tool sends: a hosted model's name, fields the engine does
/// not know, two system messages, and its own tools.
fn coding_tool_request() -> Value {
    json!({
        "model": "gpt-5",
        "stream": true,
        "user": "someone",
        "n_predict": 5,
        "top_k": 40,
        "tools": [{ "type": "function", "function": { "name": "read_file" } }],
        "messages": [
            { "role": "system", "content": "rules" },
            { "role": "user", "content": "ask" },
            { "role": "system", "content": "more" }
        ]
    })
}

/// A coding tool's request: a hosted model's name, fields the engine does not
/// know, two system messages, and its own tools. It is fitted to the engine
/// and the answer copied back, streamed as it was asked.
#[test]
fn a_coding_tools_request_is_fitted_and_relayed() -> Outcome {
    let stream = event_stream(&[r#"{"choices":[{"delta":{"content":"<tool_call>"}}]}"#]);
    let engine = FakeServer::start(vec![stream.clone()])?;
    let request = coding_tool_request();
    let mut client = Vec::new();

    answer(
        &mut client,
        &serde_json::to_vec(&request)?,
        &upstreams(&engine, &idle()?),
    )?;

    let seen = engine.requests()?;
    let sent: Value = serde_json::from_str(seen[0].split("\r\n\r\n").nth(1).unwrap_or_default())?;
    let merged = json!([
        { "role": "system", "content": "rules\n\nmore" },
        { "role": "user", "content": "ask" }
    ]);
    let body = stream.split("\r\n\r\n").nth(1).unwrap_or_default();

    assert_eq!(sent["model"], "nemotron");
    assert_eq!(sent["top_k"], 40);
    assert_eq!(sent["tools"], request["tools"]);
    assert_eq!(sent.get("user"), None);
    assert_eq!(sent.get("n_predict"), None);
    assert_eq!(sent[MESSAGES], merged);
    assert!(String::from_utf8_lossy(&client).ends_with(body));

    Ok(())
}

#[test]
fn a_named_model_is_answered_by_its_own_engine() -> Outcome {
    let qwen = FakeServer::start(vec![json_response(r#"{"choices":[]}"#)])?;
    let mut upstreams = upstreams(&idle()?, &idle()?);

    upstreams
        .others
        .push(Engine::edge("qwen".to_string(), qwen.address()));

    let request = br#"{"model":"qwen","messages":[]}"#;

    answer(&mut Vec::new(), request, &upstreams)?;

    assert_eq!(qwen.requests()?.len(), 1);

    Ok(())
}

/// llama-server understands fields an engine refuses, and answers to the
/// names it gave its models, so a request for it is passed on as it came.
#[test]
fn llama_server_gets_a_request_as_it_came() -> Outcome {
    let loaded = json_response(r#"{"data":[{"id":"nemotron"}]}"#);
    let llama = FakeServer::start(vec![loaded, json_response(r#"{"choices":[]}"#)])?;
    let mut upstreams = upstreams(&idle()?, &idle()?);
    upstreams.default = Engine::llama(llama.address(), None);

    let request = br#"{"model":"nemotron","n_predict":5,"grammar":"x","messages":[
        {"role":"user","content":"a"},{"role":"system","content":"b"}]}"#;

    answer(&mut Vec::new(), request, &upstreams)?;

    let seen = llama.requests()?;
    let sent: Value = serde_json::from_str(seen[1].split("\r\n\r\n").nth(1).unwrap_or_default())?;
    let expected: Value = serde_json::from_slice(request)?;

    assert!(seen[0].starts_with("GET /v1/models"), "{}", seen[0]);
    assert_eq!(sent, expected);

    Ok(())
}

#[test]
fn a_chunk_with_a_finish_reason_or_more_than_text_carries_more() {
    let text = json!({ "choices": [{ "delta": { "content": "hi" }, "finish_reason": null }] });
    let finished = json!({ "choices": [{ "delta": { "content": "" }, "finish_reason": "stop" }] });
    let reasoning = json!({ "choices": [{ "delta": { "reasoning_content": "hm" } }] });

    assert!(!carries_more(&text));
    assert!(carries_more(&finished));
    assert!(carries_more(&reasoning));
}
