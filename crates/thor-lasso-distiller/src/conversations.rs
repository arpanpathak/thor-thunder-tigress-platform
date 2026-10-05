//! Conversations built from book passages.
//!
//! The teacher model writes only the questions. Each answer is the book's own
//! section text, so the answers stay human-written and correct; the model
//! cannot add slop or mistakes to them. A chapter's sections, in order, become
//! the turns of one conversation, so a follow-up question leads into the next
//! section the way a reader's next question would.
//!
//! ```text
//!   chapter sections:  S1      S2      S3      S4
//!   conversations:    [Q1 S1 · Q2 S2 · Q3 S3] [Q1 S4]      (turns = 3)
//! ```
//!
//! A question is kept only when it reads like something an engineer would ask:
//! one line, ending in `?`, not mentioning "the passage" or "the book", and
//! with none of the slop phrases Stage 0 knows.

use std::{collections::BTreeMap, fmt, fs, path::Path};

use serde_json::{Value, json};
use thor_spark_safety_eval::slop;

use crate::{
    client::{Client, Message},
    error::DistillError,
};

/// How many times the model is asked for one question before the
/// conversation ends at the turn before it.
const ATTEMPTS: usize = 2;

/// The longest part of a passage shown to the model, in characters. Enough
/// for the model to see what the section is about without filling its context.
const MAX_PROMPT_PASSAGE: usize = 6_000;

/// The longest question kept, in characters.
const MAX_QUESTION: usize = 220;

/// Words that show a question was written about the text instead of about the
/// engineer's problem.
const SOURCE_WORDS: [&str; 10] = [
    "passage", "this section", "the section", "the text", "this chapter", "the chapter",
    "the book", "the author", "the excerpt", "the document",
];

/// Phrases that tie a passage to its book. Such a passage stays in the
/// training set as text, but as a chat answer "as we saw in this chapter"
/// makes no sense, so it is not used in a conversation.
const BOOK_REFERENCES: [&str; 11] = [
    "this book", "this chapter", "previous chapter", "next chapter", "in chapter ", "as we saw",
    "we'll see", "we\u{2019}ll see", "later in this", "earlier in this", "this section",
];

/// The instructions given to the teacher before every question.
const SYSTEM_PROMPT: &str = "You write the one question a working software engineer would ask a \
senior colleague, such that the given text is the answer. Ask about the engineer's problem, in \
their words, as they would type it: plain, specific, one sentence ending with a question mark. \
Do not mention any text, passage, section, chapter, book or author. Do not quote headings. Do not \
greet, thank or explain. Reply with the question only.";

/// One book or doc section from the training file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    /// The file the section came from, such as `trpl/src/ch08-01-vectors.md`.
    pub origin: String,
    /// The section text, starting with its headings.
    pub text: String,
}

impl Passage {
    /// The book or doc folder, such as `trpl`.
    pub fn collection(&self) -> &str {
        self.origin.split('/').next().unwrap_or_default()
    }

    /// True when the section can stand as a chat answer: it does not refer to
    /// its own book, chapter or section.
    pub fn reads_as_answer(&self) -> bool {
        let lowered = self.text.to_lowercase();
        !BOOK_REFERENCES.iter().any(|phrase| lowered.contains(phrase))
    }

    /// The section as an answer: the text without its leading title lines,
    /// because an engineer answering a question does not start with a heading.
    pub fn answer(&self) -> String {
        self.text
            .lines()
            .skip_while(|line| line.starts_with('#') || line.trim().is_empty())
            .collect::<Vec<&str>>()
            .join("\n")
            .trim()
            .to_string()
    }
}

/// Every passage of the training file that can stand as a chat answer, in
/// file order: the records with an empty instruction, minus those that refer
/// to their own book. With `collections` non-empty, only those folders.
pub fn read_passages(path: &Path, collections: &[String]) -> Result<Vec<Passage>, DistillError> {
    let text = fs::read_to_string(path).map_err(DistillError::io(path))?;
    let mut passages = Vec::new();
    for (index, line) in text.lines().enumerate().filter(|(_, line)| !line.trim().is_empty()) {
        let record: Value = serde_json::from_str(line).map_err(|source| DistillError::Json {
            path: path.to_path_buf(),
            line: index + 1,
            source,
        })?;
        let field = |name: &str| record.get(name).and_then(Value::as_str).unwrap_or_default();
        if !field("instruction").is_empty() {
            continue;
        }
        let passage = Passage {
            origin: field("origin").to_string(),
            text: field("response").to_string(),
        };
        let wanted = collections.is_empty() || collections.iter().any(|wanted| wanted == passage.collection());
        if wanted && passage.reads_as_answer() {
            passages.push(passage);
        }
    }
    Ok(passages)
}

/// Cuts each chapter's sections, in order, into conversations of at most
/// `turns` sections.
pub fn plan(passages: Vec<Passage>, turns: usize) -> Vec<Vec<Passage>> {
    let mut chapters: Vec<Vec<Passage>> = Vec::new();
    for passage in passages {
        match chapters.last_mut() {
            Some(chapter) if chapter.first().is_some_and(|first| first.origin == passage.origin) => {
                chapter.push(passage)
            }
            _ => chapters.push(vec![passage]),
        }
    }
    chapters
        .into_iter()
        .flat_map(|chapter| {
            chapter
                .chunks(turns.max(1))
                .map(<[Passage]>::to_vec)
                .collect::<Vec<Vec<Passage>>>()
        })
        .collect()
}

/// The messages that ask the teacher for the next question.
pub fn question_prompt(previous_questions: &[String], passage: &Passage) -> Vec<Message> {
    let shown: String = passage.text.chars().take(MAX_PROMPT_PASSAGE).collect();
    let earlier = match previous_questions.is_empty() {
        true => "This is the first question of the conversation.".to_string(),
        false => format!(
            "The engineer already asked, in order:\n{}\nWrite their next, follow-up question.",
            previous_questions
                .iter()
                .map(|question| format!("- {question}"))
                .collect::<Vec<String>>()
                .join("\n")
        ),
    };
    vec![
        Message {
            role: "system",
            content: SYSTEM_PROMPT.to_string(),
        },
        Message {
            role: "user",
            content: format!("{earlier}\n\nThe answer will be this text:\n<<<\n{shown}\n>>>"),
        },
    ]
}

/// Why a question the teacher wrote was not kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// Nothing usable came back.
    Empty,
    /// More than one line.
    NotOneLine,
    /// It does not end with a question mark.
    NotAQuestion,
    /// Longer than [`MAX_QUESTION`].
    TooLong,
    /// It talks about the text instead of the problem.
    MentionsSource(&'static str),
    /// It contains a slop phrase.
    Slop(String),
}

impl fmt::Display for Rejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rejection::Empty => formatter.write_str("empty reply"),
            Rejection::NotOneLine => formatter.write_str("more than one line"),
            Rejection::NotAQuestion => formatter.write_str("does not end with a question mark"),
            Rejection::TooLong => write!(formatter, "longer than {MAX_QUESTION} characters"),
            Rejection::MentionsSource(word) => write!(formatter, "mentions \"{word}\""),
            Rejection::Slop(phrase) => write!(formatter, "slop phrase \"{phrase}\""),
        }
    }
}

impl std::error::Error for Rejection {}

/// The question in `reply`, tidied, or why it cannot be used. Surrounding
/// quotes and a leading `Question:` are removed first, since models add them.
pub fn check_question(reply: &str) -> Result<String, Rejection> {
    let trimmed = reply.trim();
    let unlabelled = trimmed
        .strip_prefix("Question:")
        .or_else(|| trimmed.strip_prefix("Q:"))
        .unwrap_or(trimmed)
        .trim()
        .trim_matches(['"', '\u{201c}', '\u{201d}', '\''])
        .trim();
    let lowered = unlabelled.to_lowercase();
    let mentioned = SOURCE_WORDS.iter().find(|word| lowered.contains(*word));
    let slop_hit = slop::check(unlabelled).hits.into_iter().next();
    match unlabelled {
        "" => Err(Rejection::Empty),
        question if question.lines().count() > 1 => Err(Rejection::NotOneLine),
        question if !question.ends_with('?') => Err(Rejection::NotAQuestion),
        question if question.chars().count() > MAX_QUESTION => Err(Rejection::TooLong),
        _ if mentioned.is_some() => Err(Rejection::MentionsSource(mentioned.copied().unwrap_or_default())),
        _ if slop_hit.is_some() => Err(Rejection::Slop(slop_hit.map(|hit| hit.text).unwrap_or_default())),
        question => Ok(question.to_string()),
    }
}

/// One finished conversation and the questions that were thrown away on the way.
#[derive(Debug, Clone, Default)]
pub struct Conversation {
    /// The chapter file the sections came from.
    pub origin: String,
    /// Each kept question and the passage that answers it.
    pub turns: Vec<(String, String)>,
    /// Each rejected question with the reason, for review.
    pub rejected: Vec<(String, String)>,
}

impl Conversation {
    /// The conversation as one line of `conversations.jsonl`. The `id`,
    /// `source` and `origin` fields are the ones the review page indexes.
    pub fn to_json(&self, model: &str) -> Value {
        let messages: Vec<Value> = self
            .turns
            .iter()
            .flat_map(|(question, answer)| {
                [
                    json!({ "role": "user", "content": question }),
                    json!({ "role": "assistant", "content": answer }),
                ]
            })
            .collect();
        let rejected: Vec<Value> = self
            .rejected
            .iter()
            .map(|(question, reason)| json!({ "question": question, "reason": reason }))
            .collect();
        json!({
            "id": fnv_hex(&serde_json::to_string(&messages).unwrap_or_default()),
            "source": "conversation",
            "origin": self.origin,
            "model": model,
            "messages": messages,
            "rejected": rejected,
        })
    }
}

/// Asks `client` for one question per section, in order, and pairs each with
/// the section as its answer. A question that fails [`check_question`] is
/// asked again; after [`ATTEMPTS`] failures the conversation ends with the
/// turns it has.
pub fn converse(client: &Client, sections: &[Passage]) -> Result<Conversation, DistillError> {
    let mut conversation = Conversation {
        origin: sections.first().map(|passage| passage.origin.clone()).unwrap_or_default(),
        ..Conversation::default()
    };
    for passage in sections {
        let asked: Vec<String> = conversation.turns.iter().map(|(question, _)| question.clone()).collect();
        let prompt = question_prompt(&asked, passage);
        let mut question = None;
        for _ in 0..ATTEMPTS {
            let reply = client.complete(&prompt, 80, 0.7)?;
            match check_question(&reply) {
                Ok(checked) => {
                    question = Some(checked);
                    break;
                }
                Err(reason) => conversation.rejected.push((reply.trim().to_string(), reason.to_string())),
            }
        }
        match question {
            Some(question) => conversation.turns.push((question, passage.answer())),
            None => break,
        }
    }
    Ok(conversation)
}

/// How many planned conversations and turns each collection has.
pub fn plan_counts(planned: &[Vec<Passage>]) -> BTreeMap<String, (usize, usize)> {
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for conversation in planned {
        if let Some(first) = conversation.first() {
            let entry = counts.entry(first.collection().to_string()).or_default();
            entry.0 += 1;
            entry.1 += conversation.len();
        }
    }
    counts
}

/// The FNV-1a hash of `text` as 16 hex digits, the id scheme the training
/// file uses.
fn fnv_hex(text: &str) -> String {
    let hash = text
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(check_question("What does this section say about traits?"), Err(Rejection::MentionsSource("this section")));
        assert_eq!(check_question("Explain vectors."), Err(Rejection::NotAQuestion));
        assert_eq!(check_question("Great question! Why use Arc?"), Err(Rejection::Slop("Great question".to_string())));
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
        assert_eq!(passage("x", "# Vectors\n\n## Growing\n\nPush appends.").answer(), "Push appends.");
    }

    #[test]
    fn retries_a_rejected_question_and_answers_with_the_book_text() -> Result<(), DistillError> {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };
        let listener = TcpListener::bind("127.0.0.1:0").map_err(DistillError::io("listener"))?;
        let address = listener
            .local_addr()
            .map_err(DistillError::io("listener"))?
            .to_string();
        let replies = ["What does this section explain?", "Why does push sometimes reallocate a Vec?"];
        thread::spawn(move || {
            replies.iter().for_each(|reply| {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buffer = [0u8; 16_384];
                    let _ = stream.read(&mut buffer);
                    let body = json!({ "choices": [{ "message": { "content": reply } }] }).to_string();
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
                }
            });
        });
        let client = Client {
            address,
            model: "teacher".to_string(),
        };
        let conversation = converse(&client, &[passage("trpl/v.md", "# Vectors\n\nPush may reallocate.")])?;
        assert_eq!(
            conversation.turns,
            [("Why does push sometimes reallocate a Vec?".to_string(), "Push may reallocate.".to_string())]
        );
        assert_eq!(conversation.rejected.len(), 1);
        Ok(())
    }

    #[test]
    fn a_follow_up_prompt_lists_the_earlier_questions() {
        let messages = question_prompt(&["Why Vec?".to_string()], &passage("x", "text"));
        let user = messages.last().map(|message| message.content.as_str()).unwrap_or_default();
        assert!(user.contains("- Why Vec?") && user.contains("follow-up"));
    }
}
