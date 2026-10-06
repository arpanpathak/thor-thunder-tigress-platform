//! Examples from the Claude chat export.
//!
//! ## Design
//!
//! The export is a JSON array of conversations, and each conversation is a list
//! of messages from `human` or `assistant`. The serde types below mirror only
//! the fields this module reads; every other field is ignored.
//!
//! A conversation becomes [`QuestionAnswer`] pairs in one pass:
//!
//! ```text
//!   human      ─► starts a pair, or extends a question not yet answered
//!   assistant  ─► adds to the answer of the current pair
//! ```
//!
//! Each pair then becomes an [`Example`] or a [`SkipReason`]:
//!
//! * Only the visible text of an answer is kept. Thinking, tool calls and tool
//!   results are [`Content::Hidden`] and dropped.
//! * A conversation with a safety flag is left out completely.
//! * A question about an image is left out, because the export does not
//!   include the image and the answer describes something the model never sees.

use serde::Deserialize;

use crate::{
    error::DataError,
    example::{Example, SkipReason, Source},
};

/// File endings of uploads the export names but does not include.
const IMAGE_EXTENSIONS: [&str; 6] = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".heic"];

/// One conversation in the export.
#[derive(Deserialize)]
struct Conversation {
    /// The conversation id, used in each example's `origin`.
    uuid: String,
    /// The messages in the order they were sent.
    chat_messages: Vec<Message>,
}

/// One message in a conversation.
#[derive(Deserialize)]
struct Message {
    /// Who sent it.
    sender: Sender,
    /// The blocks the message is made of.
    content: Vec<Content>,
    /// Text files the user attached, with their extracted text.
    #[serde(default)]
    attachments: Vec<Attachment>,
    /// Every uploaded file, including images whose content is not exported.
    #[serde(default)]
    files: Vec<UploadedFile>,
}

/// The two parties of a conversation.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Sender {
    Human,
    Assistant,
}

/// One block of a message, told apart by its `type` field.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Content {
    /// Text shown in the chat.
    Text { text: String },
    /// A safety flag raised on the conversation.
    Flag {},
    /// Thinking, tool calls, tool results and anything else not shown as text.
    #[serde(other)]
    Hidden,
}

/// A text file attached to a message.
#[derive(Deserialize)]
struct Attachment {
    file_name: String,
    #[serde(default)]
    extracted_content: String,
}

/// A file uploaded with a message; the export keeps only its name.
#[derive(Deserialize)]
struct UploadedFile {
    file_name: String,
}

/// A question and the answer that followed it, before it becomes an example.
#[derive(Default)]
struct QuestionAnswer {
    question: String,
    answer: String,
    /// True when the question came with an image the export does not include.
    asks_about_image: bool,
}

impl Conversation {
    /// True when any message carries a safety flag.
    fn has_safety_flag(&self) -> bool {
        self.chat_messages
            .iter()
            .flat_map(|message| &message.content)
            .any(|content| matches!(content, Content::Flag {}))
    }
}

impl Message {
    /// The text blocks of the message, joined as paragraphs.
    fn visible_text(&self) -> String {
        let mut text = String::new();
        for content in &self.content {
            if let Content::Text { text: paragraph } = content {
                append_paragraph(&mut text, paragraph);
            }
        }
        text
    }

    /// The extracted text of every attachment, each under its file name.
    fn attachment_text(&self) -> String {
        let mut text = String::new();
        for attachment in &self.attachments {
            let labelled = format!(
                "Attached file `{}`:\n\n{}",
                attachment.file_name, attachment.extracted_content
            );
            append_paragraph(&mut text, &labelled);
        }
        text
    }

    /// True when the message has an image and no text attachment to read instead.
    fn has_unreadable_image(&self) -> bool {
        let has_image = self.files.iter().any(|file| {
            let file_name = file.file_name.to_lowercase();
            IMAGE_EXTENSIONS
                .iter()
                .any(|extension| file_name.ends_with(extension))
        });
        has_image && self.attachments.is_empty()
    }
}

impl QuestionAnswer {
    /// A new pair whose question is `message`.
    fn asked_in(message: &Message) -> Self {
        let mut pair = QuestionAnswer::default();
        pair.add_question(message);
        pair
    }

    /// True once any answer text has arrived.
    fn is_answered(&self) -> bool {
        !self.answer.is_empty()
    }

    /// Adds a human message to the question.
    fn add_question(&mut self, message: &Message) {
        append_paragraph(&mut self.question, &message.visible_text());
        append_paragraph(&mut self.question, &message.attachment_text());
        self.asks_about_image |= message.has_unreadable_image();
    }

    /// Adds an assistant message to the answer.
    fn add_answer(&mut self, message: &Message) {
        append_paragraph(&mut self.answer, &message.visible_text());
    }

    /// The finished example, or why the pair cannot be one.
    fn into_example(self, origin: String) -> Result<Example, SkipReason> {
        match self {
            QuestionAnswer {
                question, answer, ..
            } if question.is_empty() || answer.is_empty() => Err(SkipReason::EmptyTurn),
            QuestionAnswer {
                asks_about_image: true,
                ..
            } => Err(SkipReason::AboutImage),
            QuestionAnswer {
                question,
                answer,
                asks_about_image: false,
            } => Ok(Example {
                instruction: question,
                response: answer,
                source: Source::Chat,
                origin,
            }),
        }
    }
}

/// Reads `conversations.json` and turns every question and answer into an example.
pub fn examples(
    chat_export: &str,
    skip_reasons: &mut Vec<SkipReason>,
) -> Result<Vec<Example>, DataError> {
    let conversations: Vec<Conversation> = serde_json::from_str(chat_export)?;
    let mut examples = Vec::new();
    for conversation in conversations {
        let pairs = question_answer_pairs(&conversation.chat_messages);
        if conversation.has_safety_flag() {
            skip_reasons.extend(std::iter::repeat_n(SkipReason::SafetyFlag, pairs.len()));
            continue;
        }
        for (turn, pair) in (1..).zip(pairs) {
            let origin = format!("conversations.json#{}/{turn}", conversation.uuid);
            match pair.into_example(origin) {
                Ok(example) => examples.push(example),
                Err(reason) => skip_reasons.push(reason),
            }
        }
    }
    Ok(examples)
}

/// Pairs each human question with the assistant answer that follows it.
fn question_answer_pairs(messages: &[Message]) -> Vec<QuestionAnswer> {
    let mut pairs: Vec<QuestionAnswer> = Vec::new();
    for message in messages {
        match (&message.sender, pairs.last_mut()) {
            (Sender::Human, Some(open_pair)) if !open_pair.is_answered() => {
                open_pair.add_question(message)
            }
            (Sender::Human, Some(..) | None) => pairs.push(QuestionAnswer::asked_in(message)),
            (Sender::Assistant, Some(current_pair)) => current_pair.add_answer(message),
            (Sender::Assistant, None) => {}
        }
    }
    pairs
}

/// Reads a markdown file that holds one `#Question` and one `#Answer` section.
pub fn example_from_question_file(markdown: &str, origin: &str) -> Option<Example> {
    let after_heading = markdown.strip_prefix("#Question\n")?;
    let (question, answer) = after_heading.split_once("\n#Answer\n")?;
    Some(Example {
        instruction: question.trim().to_string(),
        response: answer.trim().to_string(),
        source: Source::Chat,
        origin: origin.to_string(),
    })
}

/// Appends `paragraph` to `text` after a blank line. Empty paragraphs add nothing.
fn append_paragraph(text: &mut String, paragraph: &str) {
    let paragraph = paragraph.trim();
    if paragraph.is_empty() {
        return;
    }
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(paragraph);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_questions_in_a_row_become_one_and_an_empty_question_is_skipped() -> Result<(), DataError> {
        let export = r#"[{"uuid": "c4", "chat_messages": [
            {"sender": "human", "content": [{"type": "text", "text": "First part."}]},
            {"sender": "human", "content": [{"type": "text", "text": "Second part."}]},
            {"sender": "assistant", "content": [{"type": "text", "text": "One answer."}]},
            {"sender": "human", "content": []},
            {"sender": "assistant", "content": [{"type": "text", "text": "Answer to nothing."}]}
        ]}]"#;
        let mut skip_reasons = Vec::new();
        let found = examples(export, &mut skip_reasons)?;
        let questions: Vec<&str> = found.iter().map(|example| example.instruction.as_str()).collect();
        assert_eq!(questions, ["First part.\n\nSecond part."]);
        assert_eq!(skip_reasons, [SkipReason::EmptyTurn]);
        Ok(())
    }

    #[test]
    fn attachments_join_the_question_and_a_stray_answer_is_ignored() -> Result<(), DataError> {
        let export = r#"[{"uuid": "c3", "chat_messages": [
            {"sender": "assistant", "content": [{"type": "text", "text": "Hello before anything was asked."}]},
            {"sender": "human", "content": [{"type": "text", "text": "Review this file."}],
             "attachments": [{"file_name": "main.rs", "extracted_content": "fn main() {}"}]},
            {"sender": "assistant", "content": [{"type": "text", "text": "It does nothing yet."}]}
        ]}]"#;
        let mut skip_reasons = Vec::new();
        let found = examples(export, &mut skip_reasons)?;
        let questions: Vec<&str> = found.iter().map(|example| example.instruction.as_str()).collect();
        assert_eq!(questions, ["Review this file.\n\nAttached file `main.rs`:\n\nfn main() {}"]);
        Ok(())
    }

    const EXPORT: &str = r#"[
      {"uuid": "c1", "chat_messages": [
        {"sender": "human", "content": [{"type": "text", "text": "What is a futex?"}]},
        {"sender": "assistant", "content": [
          {"type": "thinking", "thinking": "hidden"},
          {"type": "text", "text": "A fast userspace mutex."},
          {"type": "tool_use", "name": "search"},
          {"type": "text", "text": "The kernel only steps in on contention."}
        ]},
        {"sender": "human", "content": [{"type": "text", "text": "Look at this"}],
         "files": [{"file_name": "screen.PNG"}]},
        {"sender": "assistant", "content": [{"type": "text", "text": "The picture shows a graph."}]}
      ]},
      {"uuid": "c2", "chat_messages": [
        {"sender": "human", "content": [{"type": "text", "text": "hi"}]},
        {"sender": "assistant", "content": [{"type": "flag", "flag": "self_harm_risk"}]}
      ]}
    ]"#;

    #[test]
    fn keeps_only_the_visible_answer_text() -> Result<(), DataError> {
        let mut skip_reasons = Vec::new();
        let examples = examples(EXPORT, &mut skip_reasons)?;
        let expected = Example {
            instruction: "What is a futex?".to_string(),
            response: "A fast userspace mutex.\n\nThe kernel only steps in on contention."
                .to_string(),
            source: Source::Chat,
            origin: "conversations.json#c1/1".to_string(),
        };
        assert_eq!(examples, [expected]);
        Ok(())
    }

    #[test]
    fn leaves_out_image_questions_and_flagged_conversations() -> Result<(), DataError> {
        let mut skip_reasons = Vec::new();
        examples(EXPORT, &mut skip_reasons)?;
        assert_eq!(
            skip_reasons,
            [SkipReason::AboutImage, SkipReason::SafetyFlag]
        );
        Ok(())
    }

    #[test]
    fn reads_question_file() {
        let markdown = "#Question\nWhy &x?\n\n#Answer\nBecause iter() yields references.\n";
        let response =
            example_from_question_file(markdown, "chat_0.md").map(|example| example.response);
        assert_eq!(
            response.as_deref(),
            Some("Because iter() yields references.")
        );
    }
}
