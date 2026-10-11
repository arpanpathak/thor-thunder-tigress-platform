//! Cleaning a markdown file before it becomes examples: front matter,
//! shortcodes, HTML and hard-wrapped lines.

use std::sync::LazyLock;

use regex::Regex;

/// Removes YAML front matter, Hugo shortcodes, HTML comments and link
/// references, which are site furniture rather than prose and would otherwise
/// be learned as part of an answer, and joins hard-wrapped lines back into
/// paragraphs.
pub fn clean(markdown: &str) -> String {
    let without_front_matter = strip_front_matter(markdown);
    let has_heading = without_front_matter
        .lines()
        .any(|line| line.starts_with("# ") || line.starts_with("## "));

    let titled = match (front_matter_title(markdown), has_heading) {
        (Some(title), false) => format!("# {title}\n\n{without_front_matter}"),
        _ => without_front_matter.to_string(),
    };

    let without_front_matter = titled.as_str();
    let without_comments = HTML_COMMENT
        .as_ref()
        .map_or(without_front_matter.to_string(), |comment| {
            comment.replace_all(without_front_matter, "").into_owned()
        });
    let without_comments = HEADING_ANCHOR
        .as_ref()
        .map_or(without_comments.clone(), |anchor| {
            anchor.replace_all(&without_comments, "$1").into_owned()
        });
    let stripped = without_comments
        .lines()
        .map(strip_shortcodes)
        .map(|line| strip_html(&line))
        .collect::<Vec<String>>()
        .join("\n");

    collapse_blank_lines(&tidy_prose(&stripped))
}

static HEADING_ANCHOR: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?m)^(#{1,6} .*?)\s*\{#[\w.-]+\}[ \t]*$").ok());

static HTML_COMMENT: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").ok());

static LINK_DEFINITION: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\s{0,3}\[[^\]]+\]:\s*\S+").ok());

static REFERENCE_LINK: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\[([^\]\n]+)\]\[[^\]\n]*\]").ok());

static BLOCK_START: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\s*(?:#|[-*+]\s|\d+[.)]\s|>|\||```|~~~|<)").ok());

/// Outside code blocks: drops link definitions such as `[raw]: #raw`, turns
/// reference links such as `[Raw Identifiers][raw]` into their text, and joins
/// a line that continues the paragraph or list item above it. Lines inside
/// code blocks are kept exactly, so `grid[i][j]` is never read as a link.
pub(super) fn tidy_prose(text: &str) -> String {
    let patterns = LINK_DEFINITION
        .as_ref()
        .zip(REFERENCE_LINK.as_ref())
        .zip(BLOCK_START.as_ref());

    patterns.map_or(
        text.to_string(),
        |((definition, reference), block_start)| {
            tidy_with(text, definition, reference, block_start)
        },
    )
}

/// [`tidy_prose`] with its patterns compiled.
pub(super) fn tidy_with(
    text: &str,
    definition: &Regex,
    reference: &Regex,
    block_start: &Regex,
) -> String {
    let mut lines: Vec<(bool, String)> = Vec::new();
    let mut inside_code = false;
    let mut joinable = false;

    for line in text.lines() {
        let trimmed = line.trim_start();
        let is_fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");

        if inside_code || is_fence {
            inside_code ^= is_fence;
            lines.push((true, line.to_string()));
            joinable = false;
            continue;
        }

        if definition.is_match(line) {
            continue;
        }

        let continues = joinable && !trimmed.is_empty() && !block_start.is_match(line);

        match (continues, lines.last_mut()) {
            (true, Some((_, previous))) => {
                previous.push(' ');
                previous.push_str(trimmed);
            }
            _ => lines.push((false, line.to_string())),
        }

        joinable = !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with('|');
    }

    lines
        .into_iter()
        .map(|(is_code, line)| match is_code {
            true => line,
            false => reference.replace_all(&line, "$1").into_owned(),
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// The HTML element names the sources actually use.
///
/// A tag is only removed when its name is in this list. That matters: `Vec<i32>`
/// and `<T>` are angle brackets too, and a rule that stripped every `<...>` would
/// quietly delete Rust generics out of the training data.
pub(super) const HTML_ELEMENTS: [&str; 38] = [
    "a",
    "b",
    "blockquote",
    "br",
    "caption",
    "code",
    "dd",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "iframe",
    "img",
    "li",
    "ol",
    "p",
    "pre",
    "source",
    "span",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "th",
    "thead",
    "tr",
];

/// Removes HTML tags but keeps the text between them, then decodes the entities
/// the sources use.
///
/// Book markup learned as output teaches the model to answer in HTML. One
/// reviewer flagged an example by hand for exactly this, and the same markup
/// appears in thousands, so it is removed at the source rather than reviewed one
/// example at a time.
pub(super) fn strip_html(line: &str) -> String {
    let mut kept = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(start) = rest.find('<') {
        kept.push_str(&rest[..start]);

        let Some(offset) = rest[start..].find('>') else {
            kept.push_str(&rest[start..]);
            rest = "";
            break;
        };

        kept.push_str(tag_text(&rest[start..=start + offset]));
        rest = &rest[start + offset + 1..];
    }

    kept.push_str(rest);

    decode_entities(&kept)
}

/// What stays of `tag` in the text: all of it when it is not HTML, a space
/// for a line or paragraph break, and nothing for any other element.
pub(super) fn tag_text(tag: &str) -> &str {
    let breaks = tag.starts_with("<br") || tag.starts_with("</p") || tag == "<p>";

    match (is_html_tag(tag), breaks) {
        (false, _) => tag,
        (true, true) => " ",
        (true, false) => "",
    }
}

/// True when `tag` names a known HTML element, ignoring any attributes.
pub(super) fn is_html_tag(tag: &str) -> bool {
    let body = tag
        .strip_prefix("</")
        .or_else(|| tag.strip_prefix('<'))
        .unwrap_or(tag);
    let name: String = body
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();

    !name.is_empty() && HTML_ELEMENTS.contains(&name.as_str())
}

/// Turns the handful of HTML entities the sources use back into characters.
pub(super) fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// Squeezes runs of blank lines, which removing a tag can leave behind.
pub(super) fn collapse_blank_lines(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut blanks = 0usize;

    for line in text.lines() {
        if line.trim().is_empty() {
            blanks += 1;

            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }

        kept.push(line);
    }

    kept.join("\n")
}

/// The `title:` of a leading `---` front matter block, unquoted.
pub(super) fn front_matter_title(markdown: &str) -> Option<String> {
    let mut lines = markdown.lines();

    if lines.next().map(str::trim_end) != Some("---") {
        return None;
    }

    lines
        .take_while(|line| line.trim_end() != "---")
        .find_map(|line| line.strip_prefix("title:"))
        .map(|title| title.trim().trim_matches(['"', '\'']).to_string())
        .filter(|title| !title.is_empty())
}

/// Removes a leading `---` block.
pub(super) fn strip_front_matter(markdown: &str) -> &str {
    let mut lines = markdown.lines();

    if lines.next().map(str::trim_end) != Some("---") {
        return markdown;
    }

    let mut consumed = 4;

    for line in lines {
        consumed += line.len() + 1;

        if line.trim_end() == "---" {
            return markdown.get(consumed..).unwrap_or("");
        }
    }

    markdown
}

/// Removes `{{< ... >}}` and `{{% ... %}}` from one line.
pub(super) fn strip_shortcodes(line: &str) -> String {
    let mut kept = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(start) = rest.find("{{") {
        kept.push_str(&rest[..start]);

        let closing = rest[start..]
            .find("}}")
            .map(|offset| start + offset + 2)
            .unwrap_or(rest.len());

        kept.push_str(
            &rest[start..closing]
                .chars()
                .map(|_| ' ')
                .collect::<String>(),
        );
        rest = &rest[closing..];
    }

    kept.push_str(rest);

    kept.trim_end().to_string()
}
