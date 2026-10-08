use super::*;
use crate::testing::{FakeServer, FakeWeb, event_stream, json_response};

fn upstreams(model: &FakeServer, search: &FakeServer) -> Upstreams {
    upstreams_with(model, search, FakeWeb::default())
}

fn upstreams_with(model: &FakeServer, search: &FakeServer, web: FakeWeb) -> Upstreams {
    Upstreams {
        model: Endpoint::new(model.address(), None),
        engines: Vec::new(),
        search: Endpoint::new(search.address(), None),
        web: Box::new(web),
    }
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
    assert_eq!(Mode::of(true, true), Mode::Search);
    assert_eq!(Mode::of(true, false), Mode::Search);
    assert_eq!(Mode::of(false, true), Mode::Stream);
    assert_eq!(Mode::of(false, false), Mode::Relay);
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
    fields.insert(MESSAGES.to_string(), json!([{ "role": "system", "content": "Be brief." }, { "role": "user", "content": "hi" }]));
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

#[test]
fn a_search_plan_holds_the_queries_and_the_recency() {
    let plan = |arguments: &str| {
        ToolCall {
            arguments: arguments.to_string(),
            ..ToolCall::default()
        }
        .search_plan()
    };

    assert_eq!(
        plan(r#"{"query":"  rust  "}"#),
        Some(SearchPlan {
            queries: vec!["rust".to_string()],
            range: None
        })
    );
    assert_eq!(
        plan(r#"{"query":"jobs","time_range":"week"}"#),
        Some(SearchPlan {
            queries: vec!["jobs".to_string()],
            range: Some(TimeRange::Week)
        })
    );
    assert_eq!(
        plan(r#"{"query":"jobs","time_range":"forever"}"#),
        Some(SearchPlan {
            queries: vec!["jobs".to_string()],
            range: None
        })
    );
    assert_eq!(
        plan(r#"{"query":"rust","queries":["rust","tokio","  ","axum"]}"#),
        Some(SearchPlan {
            queries: vec!["rust".to_string(), "tokio".to_string(), "axum".to_string()],
            range: None
        })
    );
    assert_eq!(
        plan(r#"{"query":"rust engineer","kind":"jobs"}"#),
        Some(SearchPlan {
            queries: vec![
                "rust engineer".to_string(),
                "rust engineer job posting".to_string(),
                "rust engineer careers".to_string(),
                "rust engineer linkedin jobs".to_string(),
            ],
            range: None
        })
    );
    assert_eq!(
        plan(r#"{"query":"synthires","kind":"people"}"#).map(|plan| plan.queries.len()),
        Some(MAX_QUERIES_PER_CALL)
    );
    assert_eq!(
        plan(r#"{"queries":["only this"]}"#),
        Some(SearchPlan {
            queries: vec!["only this".to_string()],
            range: None
        })
    );
    assert_eq!(
        plan(r#"{"queries":["a","b"],"kind":"people"}"#).map(|plan| plan.queries.len()),
        Some(MAX_QUERIES_PER_CALL)
    );
    assert_eq!(plan(r#"{"query":"  "}"#), None);
    assert_eq!(plan(r#"{"q":"rust"}"#), None);
    assert_eq!(plan("not json"), None);
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
    let call = call_event(
        "web_search",
        r#"{"query":"rust jobs","queries":["tokio jobs"]}"#,
    );
    let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
    let search = FakeServer::start(vec![
        json_response(
            r#"{"results":[{"title":"A","url":"https://a"},{"title":"B","url":"https://b"}]}"#,
        ),
        json_response(
            r#"{"results":[{"title":"B again","url":"https://b"},{"title":"C","url":"https://c"}]}"#,
        ),
    ])?;
    let mut client = Vec::new();
    answer(
        &mut client,
        br#"{"messages":[{"role":"user","content":"jobs"}],"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;

    let asked = search.requests()?;
    assert_eq!(asked.len(), 2);
    assert!(asked[0].contains("q=rust+jobs"));
    assert!(asked[1].contains("q=tokio+jobs"));

    let seen = model.requests()?;
    assert!(seen[1].contains("[1] A"), "{}", seen[1]);
    assert!(seen[1].contains("[2] B"), "{}", seen[1]);
    assert!(seen[1].contains("[3] C"), "{}", seen[1]);
    assert!(seen[1].contains("Searches used 2 of 16"), "{}", seen[1]);

    let sent = events(&client);
    assert_eq!(
        sent.iter()
            .filter(|event| event.contains(r#""search""#))
            .count(),
        2
    );
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
    let calls: Vec<String> = (1..=MAX_ROUNDS)
        .map(|round| call_event("web_search", &format!(r#"{{"query":"q{round}"}}"#)))
        .collect();
    let mut responses: Vec<String> = calls
        .iter()
        .map(|call| event_stream(&[call.as_str()]))
        .collect();
    responses.push(event_stream(&[
        r#"{"choices":[{"delta":{"content":"done"}}]}"#,
    ]));
    let model = FakeServer::start(responses)?;
    let search = FakeServer::start(vec![
        json_response(
            r#"{"results":[{"title":"Rust","url":"https://r"}]}"#
        );
        MAX_ROUNDS
    ])?;
    answer(
        &mut Vec::new(),
        br#"{"messages":[{"role":"user","content":"news"}],"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;

    let seen = model.requests()?;
    assert_eq!(seen.len(), MAX_ROUNDS + 1);
    assert!(
        seen[MAX_ROUNDS].contains("Sources found, with the numbers to cite"),
        "{}",
        seen[MAX_ROUNDS]
    );
    assert!(
        seen[MAX_ROUNDS].contains("[1] Rust — https://r"),
        "{}",
        seen[MAX_ROUNDS]
    );
    assert!(
        !seen[MAX_ROUNDS].contains(SEARCH_HINT),
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
    assert!(model.requests()?[0].ends_with(r#"{"messages":[]}"#));
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
    let call = json!({ "choices": [{ "delta": { "content": text } }] }).to_string();
    let reply = r#"{"choices":[{"delta":{"content":"Here is the answer"}}]}"#;
    let model = FakeServer::start(vec![event_stream(&[&call]), event_stream(&[reply])])?;
    let search = FakeServer::start(vec![json_response(
        r#"{"results":[{"title":"A job","url":"https://jobs.example/1","content":"hiring"}]}"#,
    )])?;
    let mut client = Vec::new();
    answer(
        &mut client,
        br#"{"messages":[{"role":"user","content":"jobs?"}],"thor_web_search":true}"#,
        &upstreams(&model, &search),
    )?;
    let seen = model.requests()?;
    assert!(seen[1].contains("[1] A job"), "{}", seen[1]);
    assert!(search.requests()?[0].contains("q=rust+jobs"));
    let sent = events(&client);
    assert!(
        !sent.iter().any(|event| event.contains("<tool_call")),
        "{sent:?}"
    );
    assert!(
        sent.iter().any(|event| event.contains("Let me look.")),
        "{sent:?}"
    );
    assert!(
        sent.iter()
            .any(|event| event.contains("Here is the answer")),
        "{sent:?}"
    );
    Ok(())
}

#[test]
fn a_tag_the_model_leaves_open_is_flushed_at_the_end_of_the_round() -> Outcome {
    let call = call_event("web_search", r#"{"query":"rust"}"#);
    let partial =
        json!({ "choices": [{ "delta": { "content": "almost <tool" } }] }).to_string();
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

#[test]
fn the_page_budget_stops_after_twelve_pages() -> Outcome {
    let links = "<a href=\"/a\">a</a><a href=\"/b\">b</a><a href=\"/c\">c</a><a href=\"/d\">d</a><a href=\"/e\">e</a><a href=\"/f\">f</a>";
    let page = |name: &str| fetched("text/html", &format!("<title>{name}</title>{links}"));
    let web = FakeWeb::new(vec![
        ("https://docs.example/a", page("A")),
        ("https://docs.example/b", page("B")),
        ("https://docs.example/c", page("C")),
        ("https://docs.example/d", page("D")),
        ("https://docs.example/e", page("E")),
        ("https://docs.example/f", page("F")),
    ]);

    let fetch_call = |url: &str| {
        call_event(
            "fetch_page_content_recursive",
            &format!(r#"{{"url":"{url}"}}"#),
        )
    };
    let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
    let mut responses = vec![
        event_stream(&[&fetch_call("https://docs.example/a")]),
        event_stream(&[&fetch_call("https://docs.example/b")]),
        event_stream(&[&fetch_call("https://docs.example/c")]),
    ];
    responses.push(event_stream(&[reply]));
    let model = FakeServer::start(responses)?;
    answer(
        &mut Vec::new(),
        br#"{"messages":[{"role":"user","content":"read https://docs.example/a https://docs.example/b https://docs.example/c"}],"thor_web_search":true}"#,
        &upstreams_with(&model, &idle()?, web),
    )?;
    let seen = model.requests()?;
    assert!(seen[3].contains("page budget is spent"), "{}", seen[3]);
    assert!(seen[3].contains("(12 pages)"), "{}", seen[3]);
    Ok(())
}

#[test]
fn a_call_on_the_answer_round_is_run() -> Outcome {
    let call = call_event("other", "{}");
    let leaked = json!({ "choices": [{ "delta": { "content": "<tool_call><function=web_search><parameter=query>x</parameter></function></tool_call>" } }] }).to_string();
    let reply = r#"{"choices":[{"delta":{"content":"the answer"}}]}"#;
    let mut responses = vec![event_stream(&[&call]); MAX_ROUNDS];
    responses.push(event_stream(&[&leaked]));
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
    assert!(
        seen[MAX_ROUNDS + 1].contains("No results."),
        "{}",
        seen[MAX_ROUNDS + 1]
    );
    assert!(search.requests()?[0].contains("q=x"));
    let sent = events(&client);
    assert!(
        !sent.iter().any(|event| event.contains("<tool_call")),
        "{sent:?}"
    );
    assert!(
        sent.iter().any(|event| event.contains("the answer")),
        "{sent:?}"
    );
    Ok(())
}

#[test]
fn text_beside_a_call_on_the_answer_round_does_not_end_the_answer() -> Outcome {
    let call = call_event("other", "{}");
    let mixed = json!({ "choices": [{ "delta": { "content": "Here are the jobs. <tool_call><function=web_search><parameter=query>x</parameter></function></tool_call>" } }] }).to_string();
    let reply =
        r#"{"choices":[{"delta":{"content":"and the links: https://jobs.example/1"}}]}"#;
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
        outcome.is_err_and(
            |error| error.to_string() == "bad request: the body must be a JSON object"
        )
    );
    Ok(())
}

#[test]
fn only_the_first_tool_round_forces_a_call() {
    assert_eq!(Phase::of_tool_round(1), Phase::Opening);
    assert_eq!(Phase::of_tool_round(2), Phase::Searching);
    assert_eq!(Phase::of_tool_round(MAX_ROUNDS), Phase::Searching);
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

fn fetched(content_type: &str, body: &str) -> crate::http::Fetched {
    crate::http::Fetched {
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
    answer(&mut client, request, &upstreams(&model, &search))?;
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
fn reads_a_cited_page_with_the_fetch_tool() -> Outcome {
    let search_call = call_event("web_search", r#"{"query":"rust"}"#);
    let fetch_call = call_event(
        "fetch_page_content_recursive",
        r#"{"url":"https://docs.example/guide"}"#,
    );
    let reply = r#"{"choices":[{"delta":{"content":"done"}}]}"#;
    let model = FakeServer::start(vec![
        event_stream(&[&search_call]),
        event_stream(&[&fetch_call]),
        event_stream(&[reply]),
    ])?;
    let search = FakeServer::start(vec![json_response(
        r#"{"results":[{"title":"Guide","url":"https://docs.example/guide","content":"see"}]}"#,
    )])?;
    let web = FakeWeb::new(vec![(
        "https://docs.example/guide",
        fetched("text/html", "<title>Guide</title><p>the answer</p>"),
    )]);
    let mut client = Vec::new();
    let request = br#"{"messages":[{"role":"user","content":"how?"}],"thor_web_search":true}"#;
    answer(&mut client, request, &upstreams_with(&model, &search, web))?;
    let seen = model.requests()?;
    assert!(seen[2].contains("the answer"), "{}", seen[2]);
    let sent = events(&client);
    assert!(
        sent.contains(
            &json!({ "thor": { "read": { "url": "https://docs.example/guide", "title": "Guide", "domain": "docs.example" } } })
                .to_string()
        ),
        "{sent:?}"
    );
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
