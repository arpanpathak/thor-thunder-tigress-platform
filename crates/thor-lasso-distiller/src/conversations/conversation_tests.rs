use super::*;
use crate::testing::FakeModel;

fn section(text: &str) -> Passage {
    Passage {
        origin: "trpl/src/ch08.md".to_string(),
        text: format!("## Heading\n\n{text}"),
    }
}

/// What the fake teacher says: a rejected reply, a kept question and its
/// check for the first section; a repeat, an unanswered one and a kept one
/// for the second.
const REPLIES: [&str; 8] = [
    "Not a question",
    "How do I grow a vector?",
    "yes",
    "How do I grow a vector?",
    "What about strings?",
    "no",
    "How do I append to a String?",
    "yes",
];

/// The conversation the fake teacher has about two sections, and how many
/// requests it took.
fn conversed() -> Result<(Conversation, usize), DistillError> {
    let model = FakeModel::start(&REPLIES)?;
    let sections = [
        section("Use push to grow a vector."),
        section("Use push_str to append to a String."),
    ];
    let conversation = converse(&model.client, &sections)?;

    Ok((conversation, model.request_count()))
}

#[test]
fn keeps_checked_questions_with_the_section_as_the_answer() -> Result<(), DistillError> {
    let (conversation, requests) = conversed()?;
    let questions: Vec<&str> = conversation
        .turns
        .iter()
        .map(|(question, _)| question.as_str())
        .collect();

    assert_eq!(
        questions,
        ["How do I grow a vector?", "How do I append to a String?"]
    );
    assert_eq!(conversation.turns[0].1, "Use push to grow a vector.");
    assert_eq!(requests, REPLIES.len());

    Ok(())
}

#[test]
fn records_every_rejection_with_its_reason() -> Result<(), DistillError> {
    let (conversation, _) = conversed()?;
    let reasons: Vec<&str> = conversation
        .rejected
        .iter()
        .map(|(_, reason)| reason.as_str())
        .collect();
    let line = conversation.to_json("teacher");

    assert_eq!(
        reasons,
        [
            "does not end with a question mark",
            "repeats an earlier question",
            "the section does not answer it"
        ]
    );
    assert_eq!(line["messages"].as_array().map(Vec::len), Some(4));
    assert_eq!(line["rejected"].as_array().map(Vec::len), Some(3));
    assert_eq!(line["model"], "teacher");

    Ok(())
}

#[test]
fn ends_the_conversation_after_three_failed_attempts() -> Result<(), DistillError> {
    let model = FakeModel::start(&["one", "two", "three"])?;
    let conversation = converse(&model.client, &[section("Text."), section("More text.")])?;

    assert!(conversation.turns.is_empty());
    assert_eq!(conversation.rejected.len(), ATTEMPTS);

    Ok(())
}

#[test]
fn describes_every_rejection() {
    let reasons: Vec<String> = [
        Rejection::Empty,
        Rejection::NotOneLine,
        Rejection::NotAQuestion,
        Rejection::TooLong,
        Rejection::MentionsSource("chapter"),
        Rejection::Slop("great question".to_string()),
        Rejection::Repeats,
        Rejection::NotAnswered,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();

    assert_eq!(reasons[3], format!("longer than {MAX_QUESTION} characters"));
    assert_eq!(reasons[4], "mentions \"chapter\"");
    assert_eq!(reasons[5], "slop phrase \"great question\"");
}

#[test]
fn checks_every_kind_of_bad_question() {
    assert_eq!(check_question("  "), Err(Rejection::Empty));
    assert_eq!(check_question("Why?\nAnd why?"), Err(Rejection::NotOneLine));
    assert_eq!(
        check_question(&format!("{}?", "a".repeat(MAX_QUESTION))),
        Err(Rejection::TooLong)
    );
    assert!(matches!(
        check_question("What does this chapter say?"),
        Err(Rejection::MentionsSource(_))
    ));
    assert!(matches!(
        check_question("Great question, how do I start?"),
        Err(Rejection::Slop(_))
    ));
    assert_eq!(
        check_question("Question: \"How do I start?\""),
        Ok("How do I start?".to_string())
    );
}

#[test]
fn a_bad_line_in_the_training_file_names_its_line() -> Result<(), DistillError> {
    let path = std::env::temp_dir().join(format!("thor-lasso-bad-{}.jsonl", std::process::id()));
    fs::write(&path, "{\"instruction\":\"\"}\nnot json\n").map_err(DistillError::io(&path))?;

    let error = read_passages(&path, &[])
        .err()
        .map(|error| error.to_string());

    fs::remove_file(&path).map_err(DistillError::io(&path))?;

    assert!(error.is_some_and(|message| message.contains(":2: ")));

    Ok(())
}

#[test]
fn overlap_of_questions_without_long_words_is_zero() {
    assert!(overlap("a b", "c d").abs() < f64::EPSILON);
}
