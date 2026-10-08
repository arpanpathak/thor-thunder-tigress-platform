//! Examples from book and doc chapters.
//!
//! ## Design
//!
//! A chapter becomes one passage per section: plain text with no question,
//! because a heading is not something anyone asks. The passage keeps the chapter
//! title and the section heading so it reads on its own:
//!
//! ```text
//!   # An LRU cache                  ─► "# An LRU cache\n\nIntro text."
//!   ## 13.1 What an LRU cache does  ─► "# An LRU cache\n\n## What an LRU cache does\n\nIt evicts."
//! ```
//!
//! The instruction is left empty, which marks the record as text to learn the
//! writing from rather than a question and answer. Questions for these
//! passages are written later, by a model, and reviewed before they are used.
//!
//! A chapter is cut at its `#` and `##` headings. A section longer than
//! [`MAX_SECTION_CHARS`] is cut again at `###`. On the real corpus that second
//! cut is enough: 4 of 674 sections are too long, and none of their
//! subsections are. Lines inside code blocks are never read as headings,
//! because `# comment` in a shell listing is not a chapter title.

use std::{mem, path::Path};

use crate::example::{Example, Source};

/// About 3,000 tokens: one section of a chapter, short enough to train on.
const MAX_SECTION_CHARS: usize = 12_000;

/// The headings that cut a chapter into sections.
const SECTION_MARKERS: [&str; 2] = ["# ", "## "];

/// The headings that cut an oversized section into subsections.
const SUBSECTION_MARKERS: [&str; 1] = ["### "];

/// A heading and the text under it, up to the next heading of the same level.
struct Section {
    heading: String,
    body: String,
}

/// What one line of markdown means to the splitter.
enum LineKind<'a> {
    /// A line opening or closing a code block.
    CodeFence,
    /// A heading at the level being cut, with its text.
    Heading(&'a str),
    /// Any other line, including headings inside code blocks.
    Text,
}

impl Section {
    /// An empty section under `heading`, with any leading numbering removed.
    fn new(heading: &str) -> Self {
        Section {
            heading: strip_numbering(heading),
            body: String::new(),
        }
    }

    /// Adds one line to the body.
    fn add_line(&mut self, line: &str) {
        self.body.push_str(line);
        self.body.push('\n');
    }

    /// True when the body holds more than whitespace.
    fn has_text(&self) -> bool {
        !self.body.trim().is_empty()
    }

    /// True when the section must be cut again at `###`.
    fn is_too_long(&self) -> bool {
        self.body.len() > MAX_SECTION_CHARS
    }
}

/// Every section of one markdown chapter as an example.
pub fn examples(chapter_markdown: &str, origin: &str) -> Vec<Example> {
    let chapter = chapter_title(chapter_markdown, origin);
    let mut examples = Vec::new();

    for section in sections(chapter_markdown, &SECTION_MARKERS, &chapter) {
        let training_sections = match section.is_too_long() {
            true => sections(&section.body, &SUBSECTION_MARKERS, &section.heading),
            false => vec![section],
        };

        for training_section in training_sections.into_iter().filter(Section::has_text) {
            examples.push(Example {
                instruction: String::new(),
                response: passage(
                    &chapter,
                    &training_section.heading,
                    training_section.body.trim(),
                ),
                source: Source::Book,
                origin: origin.to_string(),
            });
        }
    }
    examples
}

/// Cuts `markdown` at every heading that starts with one of `heading_markers`.
/// Text before the first heading goes into a section named `first_heading`.
fn sections(markdown: &str, heading_markers: &[&str], first_heading: &str) -> Vec<Section> {
    let mut finished = Vec::new();
    let mut current = Section::new(first_heading);
    let mut inside_code_block = false;

    for line in markdown.lines() {
        match line_kind(line, heading_markers, inside_code_block) {
            LineKind::CodeFence => {
                inside_code_block = !inside_code_block;
                current.add_line(line);
            }
            LineKind::Heading(heading) => {
                finished.push(mem::replace(&mut current, Section::new(heading)))
            }
            LineKind::Text => current.add_line(line),
        }
    }
    finished.push(current);
    finished
}

/// Classifies one line, given whether the splitter is inside a code block.
fn line_kind<'a>(line: &'a str, heading_markers: &[&str], inside_code_block: bool) -> LineKind<'a> {
    let is_code_fence =
        line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~");
    let heading = heading_markers
        .iter()
        .find_map(|marker| line.strip_prefix(marker));

    match (is_code_fence, inside_code_block, heading) {
        (true, ..) => LineKind::CodeFence,
        (false, false, Some(heading)) => LineKind::Heading(heading),
        (false, true, ..) | (false, false, None) => LineKind::Text,
    }
}

/// The text of the chapter's `#` heading, else its first `##` heading (some
/// books start chapters at `##`), else the file name.
fn chapter_title(chapter_markdown: &str, origin: &str) -> String {
    let first_with = |marker: &str| {
        chapter_markdown
            .lines()
            .find_map(|line| line.strip_prefix(marker))
    };
    let title_line = first_with("# ").or_else(|| first_with("## "));

    match title_line {
        Some(title) => strip_numbering(title),
        None => file_stem(origin).replace(['-', '_'], " "),
    }
}

/// The file name without folders and extension, `ch13-lru-cache` for `src/ch13-lru-cache.md`.
fn file_stem(origin: &str) -> &str {
    Path::new(origin)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(origin)
}

/// One section as a passage that reads on its own: the chapter title, the
/// section heading when it differs, then the text.
fn passage(chapter: &str, heading: &str, body: &str) -> String {
    match heading == chapter {
        true => format!("# {chapter}\n\n{body}"),
        false => format!("# {chapter}\n\n## {heading}\n\n{body}"),
    }
}

/// Removes section numbers such as `13.1` or `02:` from the start of a heading.
fn strip_numbering(heading: &str) -> String {
    let is_numbering =
        |character: char| character.is_ascii_digit() || character == '.' || character == ':';
    heading.trim_start_matches(is_numbering).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chapter_without_a_heading_is_named_after_its_file() {
        assert_eq!(
            chapter_title("Only text, no heading.", "src/ch20-memory_and-context.md"),
            "ch20 memory and context"
        );
        assert_eq!(file_stem(""), "");
    }

    const CHAPTER: &str = "# 13 An LRU cache\n\nIntro text.\n\n## 13.1 What an LRU cache does\n\nIt evicts.\n\n```bash\n# not a heading\n```\n\n## Summary\n\nWe built one.\n";

    fn first_lines(examples: &[Example]) -> Vec<String> {
        examples
            .iter()
            .map(|example| {
                example
                    .response
                    .lines()
                    .take(3)
                    .collect::<Vec<&str>>()
                    .join("|")
            })
            .collect()
    }

    #[test]
    fn cuts_at_headings_outside_code_blocks() {
        let examples = examples(CHAPTER, "src/ch13-lru-cache.md");
        let expected = [
            "# An LRU cache||Intro text.",
            "# An LRU cache||## What an LRU cache does",
            "# An LRU cache||## Summary",
        ];
        assert_eq!(first_lines(&examples), expected);
        assert!(
            examples
                .iter()
                .all(|example| example.instruction.is_empty())
        );
        assert!(examples[1].response.contains("# not a heading"));
    }

    #[test]
    fn a_chapter_that_starts_at_level_two_takes_its_title_from_there() {
        let examples = examples(
            "## Appendix A: Keywords\n\nReserved words.\n",
            "src/appendix-01-keywords.md",
        );
        assert_eq!(
            first_lines(&examples),
            ["# Appendix A: Keywords||Reserved words."]
        );
    }

    #[test]
    fn cuts_long_sections_at_level_three_headings() {
        let paragraph = "word ".repeat(3_000);
        let chapter = format!(
            "# Big\n\n## Long\n\nIntro.\n\n### First half\n\n{paragraph}\n\n### Second half\n\n{paragraph}\n"
        );
        let examples = examples(&chapter, "big.md");
        let expected = [
            "# Big||## Long",
            "# Big||## First half",
            "# Big||## Second half",
        ];
        assert_eq!(first_lines(&examples), expected);
    }
}
