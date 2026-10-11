use super::*;
use crate::testing::FakeModel;

fn passage(origin: &str, text: &str) -> Passage {
    Passage {
        origin: origin.to_string(),
        text: text.to_string(),
    }
}

#[test]
fn keeps_an_engineers_question() {
    assert_eq!(
        check_question("  \"How do I share a Vec<u8> between threads without copying it?\"\n"),
        Ok("How do I share a Vec<u8> between threads without copying it?".to_string())
    );
}

#[test]
fn rejects_questions_about_the_text_or_with_slop() {
    assert_eq!(
        check_question("What does this section say about traits?"),
        Err(Rejection::MentionsSource("this section"))
    );
    assert_eq!(
        check_question("Explain vectors."),
        Err(Rejection::NotAQuestion)
    );
    assert_eq!(
        check_question("Great question! Why use Arc?"),
        Err(Rejection::Slop("Great question".to_string()))
    );
    assert_eq!(check_question("Why?\nBecause."), Err(Rejection::NotOneLine));
}

#[test]
fn plans_conversations_per_chapter_in_order() {
    let passages = vec![
        passage("trpl/a.md", "# A\n\none"),
        passage("trpl/a.md", "# A\n\n## B\n\ntwo"),
        passage("trpl/a.md", "# A\n\n## C\n\nthree"),
        passage("trpl/b.md", "# B\n\nfour"),
    ];
    let planned = plan(passages, 2);
    let sizes: Vec<usize> = planned.iter().map(Vec::len).collect();

    assert_eq!(sizes, [2, 1, 1]);
    assert_eq!(plan_counts(&planned).get("trpl"), Some(&(3, 4)));
}

#[test]
fn a_section_that_points_into_its_book_is_not_an_answer() {
    assert!(!passage("x", "As we saw in Chapter 3, tasks share memory.").reads_as_answer());
    assert!(passage("x", "Tasks share memory.").reads_as_answer());
}

#[test]
fn an_answer_drops_the_title_lines() {
    assert_eq!(
        passage("x", "# Vectors\n\n## Growing\n\nPush appends.").answer(),
        "Push appends."
    );
}

#[test]
fn retries_a_rejected_question_and_answers_with_the_book_text() -> Result<(), DistillError> {
    let model = FakeModel::start(&[
        "What does this section explain?",
        "Why does push sometimes reallocate a Vec?",
        "yes",
    ])?;
    let section = passage("trpl/v.md", "# Vectors\n\nPush may reallocate.");
    let conversation = converse(&model.client, &[section])?;

    assert_eq!(
        conversation.turns,
        [(
            "Why does push sometimes reallocate a Vec?".to_string(),
            "Push may reallocate.".to_string()
        )]
    );
    assert_eq!(conversation.rejected.len(), 1);

    Ok(())
}

#[test]
fn measures_how_much_two_questions_overlap() {
    assert!(
        overlap(
            "Why does Vec push reallocate?",
            "Why does push on a Vec reallocate?"
        ) >= REPEAT_OVERLAP
    );
    assert!(
        overlap(
            "Why does Vec push reallocate?",
            "How do I share an Arc between threads?"
        ) < REPEAT_OVERLAP
    );
}

#[test]
fn a_follow_up_prompt_lists_the_earlier_questions() {
    let messages = question_prompt(&["Why Vec?".to_string()], &passage("x", "text"));
    let user = messages
        .last()
        .map(|message| message.content.as_str())
        .unwrap_or_default();

    assert!(user.contains("- Why Vec?") && user.contains("follow-up"));
}
