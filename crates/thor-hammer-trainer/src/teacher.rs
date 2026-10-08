//! Conversations the teacher wrote, from `train/teacher/*.md`.
//!
//! ## Format
//!
//! ```text
//!   <!-- source: std::collections::BinaryHeap -->
//!   ### User
//!   Write a running median in Rust.
//!
//!   ### Assistant
//!   ...code and a short explanation...
//!
//!   ### User
//!   No unwrap, please.
//!
//!   ### Assistant
//!   ...the same code without unwrap...
//!
//!   ### Rejected
//!   ...an answer the last one is preferred to...
//!   ---
//! ```
//!
//! Entries are separated by a line holding only `---`. Each starts with a
//! `source` comment naming the document, book or library it is based on. An
//! entry written from one real section of the corpus names it too:
//!
//! ```text
//!   <!-- source: trpl/src/ch08-02-strings.md; section: 9f3c2a71d04e8b65; licence: Apache-2.0 -->
//! ```
//!
//! `section` is the id of that section in `data/train.jsonl`. Turns
//! alternate, start with the user and end with the assistant. `### Rejected` is
//! optional and comes last; an entry with it is also a preference pair.

use std::fmt;

use serde::Serialize;

use crate::example::stable_id;

/// Opens the comment that names an entry's source.
const SOURCE_START: &str = "<!-- source:";

/// Closes the source comment.
const COMMENT_END: &str = "-->";

/// Separates two entries.
const ENTRY_SEPARATOR: &str = "\n---\n";

/// Who speaks in a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The person asking.
    User,
    /// The model being trained.
    Assistant,
}

/// One message of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Turn {
    /// Who speaks.
    pub role: Role,
    /// What they say, as markdown.
    pub content: String,
}

/// One entry: a conversation to learn, and optionally a worse last answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    /// The document, book or library the entry is based on.
    pub source: String,
    /// The id of the real section in `data/train.jsonl` the entry was written
    /// from, when it was written from one.
    pub section: Option<String>,
    /// The licence of that section's source.
    pub licence: Option<String>,
    /// The file and entry number, so a reviewer can find it.
    pub origin: String,
    /// The turns, user first, assistant last.
    pub turns: Vec<Turn>,
    /// An answer the last assistant turn is preferred to.
    pub rejected: Option<String>,
}

impl Conversation {
    /// A stable id: [`stable_id`] of every turn's text.
    #[must_use]
    pub fn id(&self) -> String {
        let text: Vec<&str> = self
            .turns
            .iter()
            .map(|turn| turn.content.as_str())
            .collect();
        stable_id(&text.join("\n"))
    }

    /// The turns before the last answer: the prompt of a preference pair.
    #[must_use]
    pub fn prompt(&self) -> &[Turn] {
        self.turns.split_last().map_or(&[], |(_, before)| before)
    }

    /// The last answer: the chosen side of a preference pair.
    #[must_use]
    pub fn last_answer(&self) -> Option<&Turn> {
        self.turns.last()
    }

    /// The assistant turns with their position in the conversation, from 1.
    pub fn answers(&self) -> impl Iterator<Item = (usize, &Turn)> {
        self.turns
            .iter()
            .enumerate()
            .filter(|(_, turn)| turn.role == Role::Assistant)
            .map(|(position, turn)| (position + 1, turn))
    }

    /// Characters in every turn and the rejected answer.
    #[must_use]
    pub fn char_count(&self) -> usize {
        let turns: usize = self
            .turns
            .iter()
            .map(|turn| turn.content.chars().count())
            .sum();
        turns
            + self
                .rejected
                .as_ref()
                .map_or(0, |rejected| rejected.chars().count())
    }
}

/// Why an entry does not follow the format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// The entry does not start with a `source` comment.
    NoSource,
    /// The entry has no turns.
    NoTurns,
    /// Two turns in a row have the same speaker, or the first is not the user's.
    OutOfOrder,
    /// The last turn is the user's.
    EndsWithUser,
    /// `### Rejected` is not the last section, or appears twice.
    RejectedNotLast,
    /// A section is empty.
    EmptySection,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            FormatError::NoSource => "no <!-- source: ... --> comment at the start",
            FormatError::NoTurns => "no ### User or ### Assistant section",
            FormatError::OutOfOrder => "turns must alternate, starting with ### User",
            FormatError::EndsWithUser => "the last turn must be ### Assistant",
            FormatError::RejectedNotLast => "### Rejected must come once, after the last turn",
            FormatError::EmptySection => "a section is empty",
        };
        f.write_str(message)
    }
}

impl std::error::Error for FormatError {}

/// A section heading of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Heading {
    Speaker(Role),
    Rejected,
}

impl Heading {
    fn of(line: &str) -> Option<Heading> {
        match line.trim_end() {
            "### User" => Some(Heading::Speaker(Role::User)),
            "### Assistant" => Some(Heading::Speaker(Role::Assistant)),
            "### Rejected" => Some(Heading::Rejected),
            _ => None,
        }
    }
}

/// Every entry of one file, each read or refused with its origin.
#[must_use]
pub fn entries(markdown: &str, file: &str) -> Vec<(String, Result<Conversation, FormatError>)> {
    markdown
        .split(ENTRY_SEPARATOR)
        .filter(|entry| !entry.trim().is_empty())
        .enumerate()
        .map(|(position, entry)| {
            let origin = format!("{file}#{}", position + 1);
            let conversation = parse(entry, &origin);
            (origin, conversation)
        })
        .collect()
}

/// Reads one entry.
///
/// # Errors
///
/// A [`FormatError`] naming the first thing the entry gets wrong.
pub fn parse(entry: &str, origin: &str) -> Result<Conversation, FormatError> {
    let (comment, body) = source_and_body(entry).ok_or(FormatError::NoSource)?;
    let Provenance {
        source,
        section,
        licence,
    } = Provenance::read(&comment);
    let sections = sections(body);
    let (rejected, turn_sections) = match sections.split_last() {
        Some(((Heading::Rejected, text), before)) => (Some(text.clone()), before),
        _ => (None, sections.as_slice()),
    };
    let turns = turn_sections
        .iter()
        .map(|(heading, content)| match heading {
            Heading::Speaker(role) => Ok(Turn {
                role: *role,
                content: content.clone(),
            }),
            Heading::Rejected => Err(FormatError::RejectedNotLast),
        })
        .collect::<Result<Vec<Turn>, FormatError>>()?;
    check_order(&turns)?;

    if turns.iter().any(|turn| turn.content.is_empty())
        || rejected.as_ref().is_some_and(String::is_empty)
    {
        return Err(FormatError::EmptySection);
    }

    Ok(Conversation {
        source,
        section,
        licence,
        origin: origin.to_string(),
        turns,
        rejected,
    })
}

/// What the source comment says about where an entry comes from.
struct Provenance {
    source: String,
    section: Option<String>,
    licence: Option<String>,
}

impl Provenance {
    /// Reads `source; section: id; licence: name`; only the source is required.
    fn read(comment: &str) -> Provenance {
        let mut parts = comment.split(';').map(str::trim);
        let source = parts.next().unwrap_or_default().to_string();
        let mut provenance = Provenance {
            source,
            section: None,
            licence: None,
        };

        for (key, value) in parts.filter_map(|part| part.split_once(':')) {
            let value = Some(value.trim().to_string());

            match key.trim() {
                "section" => provenance.section = value,
                "licence" => provenance.licence = value,
                _ => {}
            }
        }

        provenance
    }
}

fn source_and_body(entry: &str) -> Option<(String, &str)> {
    let after_start = entry.trim_start().strip_prefix(SOURCE_START)?;
    let (source, body) = after_start.split_once(COMMENT_END)?;
    Some((source.trim().to_string(), body))
}

fn sections(body: &str) -> Vec<(Heading, String)> {
    let mut sections: Vec<(Heading, Vec<&str>)> = Vec::new();

    for line in body.lines() {
        match (Heading::of(line), sections.last_mut()) {
            (Some(heading), _) => sections.push((heading, Vec::new())),
            (None, Some((_, lines))) => lines.push(line),
            (None, None) => {}
        }
    }

    sections
        .into_iter()
        .map(|(heading, lines)| (heading, lines.join("\n").trim().to_string()))
        .collect()
}

fn check_order(turns: &[Turn]) -> Result<(), FormatError> {
    let Some(last) = turns.last() else {
        return Err(FormatError::NoTurns);
    };

    let alternates = turns
        .iter()
        .zip([Role::User, Role::Assistant].iter().cycle())
        .all(|(turn, role)| turn.role == *role);

    match (alternates, last.role) {
        (false, _) => Err(FormatError::OutOfOrder),
        (true, Role::User) => Err(FormatError::EndsWithUser),
        (true, Role::Assistant) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "<!-- source: std docs -->\n### User\nWrite it.\n\n### Assistant\nDone.\n\n### User\nNo unwrap.\n\n### Assistant\nFixed.\n\n### Rejected\nStill unwraps.\n";

    #[test]
    fn reads_turns_and_the_rejected_answer() -> Result<(), FormatError> {
        let conversation = parse(ENTRY, "a.md#1")?;
        assert_eq!(conversation.source, "std docs");
        assert_eq!(conversation.turns.len(), 4);
        assert_eq!(conversation.prompt().len(), 3);
        assert_eq!(
            conversation.last_answer().map(|turn| turn.content.as_str()),
            Some("Fixed.")
        );
        assert_eq!(conversation.rejected.as_deref(), Some("Still unwraps."));
        assert_eq!(
            conversation
                .answers()
                .map(|(position, _)| position)
                .collect::<Vec<_>>(),
            [2, 4]
        );
        Ok(())
    }

    #[test]
    fn reads_the_section_and_licence_of_a_grounded_entry() -> Result<(), FormatError> {
        let entry = "<!-- source: trpl/src/ch08.md; section: abc123; licence: Apache-2.0 -->\n### User\nQ\n### Assistant\nA\n";
        let conversation = parse(entry, "x")?;
        assert_eq!(conversation.source, "trpl/src/ch08.md");
        assert_eq!(conversation.section.as_deref(), Some("abc123"));
        assert_eq!(conversation.licence.as_deref(), Some("Apache-2.0"));
        let plain = parse(ENTRY, "x")?;
        assert_eq!((plain.section, plain.licence), (None, None));
        Ok(())
    }

    #[test]
    fn splits_a_file_into_numbered_entries() {
        let file = format!("{ENTRY}---\n<!-- source: b -->\n### User\nQ\n### Assistant\nA\n");
        let origins: Vec<String> = entries(&file, "f.md")
            .into_iter()
            .map(|(origin, _)| origin)
            .collect();
        assert_eq!(origins, ["f.md#1", "f.md#2"]);
    }

    #[test]
    fn refuses_entries_that_break_the_format() {
        let refused = |entry: &str| parse(entry, "x").err();
        assert_eq!(
            refused("### User\nQ\n### Assistant\nA"),
            Some(FormatError::NoSource)
        );
        assert_eq!(
            refused("<!-- source: s -->\nno sections"),
            Some(FormatError::NoTurns)
        );
        assert_eq!(
            refused("<!-- source: s -->\n### Assistant\nA"),
            Some(FormatError::OutOfOrder)
        );
        assert_eq!(
            refused("<!-- source: s -->\n### User\nQ"),
            Some(FormatError::EndsWithUser)
        );
        assert_eq!(
            refused("<!-- source: s -->\n### User\nQ\n### Rejected\nR\n### Assistant\nA"),
            Some(FormatError::RejectedNotLast)
        );
        assert_eq!(
            refused("<!-- source: s -->\n### User\n\n### Assistant\nA"),
            Some(FormatError::EmptySection)
        );
    }

    #[test]
    fn describes_every_format_error_and_ignores_unknown_keys() -> Result<(), FormatError> {
        let errors = [
            FormatError::NoSource,
            FormatError::NoTurns,
            FormatError::OutOfOrder,
            FormatError::EndsWithUser,
            FormatError::RejectedNotLast,
            FormatError::EmptySection,
        ];
        assert!(errors.iter().all(|error| !error.to_string().is_empty()));
        let text = "<!-- source: s; topic: heaps -->\n### User\nQ\n### Assistant\nA";
        assert_eq!(parse(text, "x")?.section, None);
        assert_eq!(parse("<!-- source: s -->\n### User\nQ\n### Assistant\nA\n### Rejected\nR\n### Rejected\nR", "x").err(), Some(FormatError::RejectedNotLast));
        Ok(())
    }

    #[test]
    fn the_id_depends_only_on_the_turns() -> Result<(), FormatError> {
        let first = parse(ENTRY, "a.md#1")?;
        let moved = parse(ENTRY, "b.md#9")?;
        assert_eq!(first.id(), moved.id());
        assert_eq!(first.id().len(), 16);
        Ok(())
    }

    #[test]
    fn counts_characters_with_the_rejected_answer() -> Result<(), FormatError> {
        let conversation = parse(ENTRY, "a")?;
        assert_eq!(
            conversation.char_count(),
            "Write it.Done.No unwrap.Fixed.Still unwraps.".len()
        );
        Ok(())
    }
}
