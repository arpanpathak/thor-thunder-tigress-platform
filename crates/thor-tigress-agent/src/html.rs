//! Turning a downloaded page into something the model can read: its title, its
//! text, and the links on its own site.
//!
//! ```text
//! <html><head><title>A</title></head>
//!   <body><p>text</p><a href="/b">B</a></body></html>
//!        │
//!        ├── title() ──► "A"
//!        ├── to_text() ──► "text\n\nB"
//!        └── links() ──► https://site/b  (same site only, deduplicated)
//! ```

use crate::address::Url;
use crate::error::{AgentError, Outcome};

/// How wide html2text is told to lay the text out.
const WIDTH: usize = 100;

/// The most links one page may offer for a crawl.
const MAX_LINKS: usize = 24;

/// The most characters a title keeps.
const MAX_TITLE: usize = 200;

/// The text of an HTML page: scripts and styles dropped, code, lists and
/// tables kept, and no run of more than one blank line.
///
/// # Errors
///
/// [`AgentError::Fetch`] when the document cannot be parsed.
pub fn to_text(html: &str) -> Outcome<String> {
    let text = html2text::from_read(html.as_bytes(), WIDTH)
        .map_err(|error| AgentError::Fetch(format!("html: {error}")))?;
    Ok(tidy(&text))
}

/// The page's `<title>`, with its entities decoded; `None` when there is none.
#[must_use]
pub fn title(html: &str) -> Option<String> {
    let lowered = html.to_ascii_lowercase();
    let start = lowered.find("<title")?;
    let open = lowered[start..].find('>')? + start + 1;
    let end = lowered[open..].find("</title")? + open;
    let text = decode(&html[open..end]);
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.chars().take(MAX_TITLE).collect())
}

/// The `https` links on `base`'s own site, in the order they appear and
/// without duplicates.
#[must_use]
pub fn links(html: &str, base: &Url) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for raw in hrefs(html) {
        if found.len() >= MAX_LINKS {
            break;
        }
        let Some(url) = resolve(base, &decode(&raw)) else {
            continue;
        };
        if !found.contains(&url) {
            found.push(url);
        }
    }
    found
}

/// Cuts `text` to at most `max` characters, at a line break when there is one.
#[must_use]
pub fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let marker = "\n[...]\n";
    let budget = max.saturating_sub(marker.len() + 1);
    let head: String = text.chars().take(budget).collect();
    let kept = head
        .rsplit_once('\n')
        .map_or(head.as_str(), |(before, _)| before);
    format!("{}{marker}", kept.trim_end())
}

/// Every `href` value in the document, without its quotes.
fn hrefs(html: &str) -> Vec<String> {
    let lowered = html.to_ascii_lowercase();
    let mut values = Vec::new();
    let mut from = 0;
    while let Some(at) = lowered[from..].find("href") {
        let at = from + at;
        let after = at + "href".len();
        let rest = &html[after..];
        let Some(value) = quoted(rest) else {
            from = after;
            continue;
        };
        values.push(value.to_string());
        from = after + value.len();
    }
    values
}

/// The quoted text just after `href`, skipping spaces and an `=`.
fn quoted(rest: &str) -> Option<&str> {
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    let closing = match quote {
        '"' => '"',
        '\'' => '\'',
        _ => return None,
    };
    let body = rest.get(1..)?;
    let end = body.find(closing)?;
    body.get(..end)
}

/// Resolves a link found on `base` to an address on the same site.
fn resolve(base: &Url, raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') {
        return None;
    }
    let lowered = raw.to_ascii_lowercase();
    if ["mailto:", "tel:", "data:", "javascript:"]
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
    {
        return None;
    }
    let url = base.absolute(raw)?;
    url.is_same_site(base).then_some(url)
}

/// Collapses runs of blank lines to one and trims the ends.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blanks = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

/// Decodes the entities a page is likely to use.
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        match named(entity) {
            Some(character) => out.push_str(&character),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The character for a named or numeric entity.
fn named(entity: &str) -> Option<String> {
    let character = match entity {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" | "#39" => '\'',
        "nbsp" => ' ',
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        other => {
            let digits = other.strip_prefix('#')?;
            let code = if let Some(hex) = digits
                .strip_prefix('x')
                .or_else(|| digits.strip_prefix('X'))
            {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                digits.parse().ok()?
            };
            char::from_u32(code)?
        }
    };
    Some(character.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(raw: &str) -> Outcome<Url> {
        Url::parse(raw)
    }

    #[test]
    fn keeps_readable_text_and_drops_markup() -> Outcome {
        let html = "<html><head><title>T</title><style>p{color:red}</style></head>\
                    <body><h1>Head</h1><script>alert(1)</script><p>One</p><pre>code()</pre></body></html>";
        let text = to_text(html)?;
        assert!(text.contains("Head") && text.contains("One") && text.contains("code()"));
        assert!(!text.contains("color:red") && !text.contains("alert"));
        assert_eq!(to_text("<p>a</p><p></p><p></p><p>b</p>")?, "a\n\nb");
        Ok(())
    }

    #[test]
    fn reads_titles_with_entities() {
        assert_eq!(
            title("<title> Rust &amp; the web </title>").as_deref(),
            Some("Rust & the web")
        );
        assert_eq!(title("<TITLE>Hi</TITLE>").as_deref(), Some("Hi"));
        assert_eq!(title("<title></title>"), None);
        assert_eq!(title("<p>no title</p>"), None);
        assert_eq!(title("&amp; <title>"), None);
        assert_eq!(title("<title>unclosed"), None);
        assert_eq!(
            title("<title>She said &quot;hi&quot; &mdash; now &#65;&#x42;</title>").as_deref(),
            Some("She said \"hi\" — now AB")
        );
        assert_eq!(
            title(&format!("<title>{}</title>", "x".repeat(300))).map(|t| t.chars().count()),
            Some(200)
        );
    }

    #[test]
    fn decodes_edges() {
        assert_eq!(decode("a &unknown; b"), "a &unknown; b");
        assert_eq!(decode("100% & &amp"), "100% & &amp");
        assert_eq!(decode("&nbsp;"), " ");
        assert_eq!(named("&"), None);
    }

    #[test]
    fn finds_same_site_links_only() -> Outcome {
        let base = site("https://example.com/docs/start")?;
        let html = r##"<a href="/a">A</a><a href='b'>B</a><a href="?q=1">Q</a>
                      <a href="https://example.com/c">C</a><a href="https://other.com/d">D</a>
                      <a href="#top">T</a><a href="mailto:x@y">M</a><a href="/a">A again</a>
                      <a href="//example.com/e">E</a><a href="../up">U</a>"##;
        let found: Vec<String> = links(html, &base).iter().map(Url::as_string).collect();
        assert_eq!(
            found,
            [
                "https://example.com/a",
                "https://example.com/docs/b",
                "https://example.com/docs/?q=1",
                "https://example.com/c",
                "https://example.com/e",
                "https://example.com/up",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_link_without_quotes_or_a_value_is_skipped() -> Outcome {
        let base = site("https://example.com/")?;
        let found = links("<a href=x>1</a><a href>2</a><a href=\"\">3</a>", &base);
        assert_eq!(found, []);
        Ok(())
    }

    #[test]
    fn caps_the_links_it_returns() -> Outcome {
        let base = site("https://example.com/")?;
        let html: String = (1..40).map(|n| format!("<a href=\"/{n}\">x</a>")).collect();
        assert_eq!(links(&html, &base).len(), MAX_LINKS);
        Ok(())
    }

    #[test]
    fn cuts_at_a_line_break() {
        assert_eq!(cut("short", 20), "short");
        assert_eq!(cut("one\ntwo\nthree", 12), "one\n[...]\n");
        assert_eq!(cut("abcdefghij", 20), "abcdefghij");
        let long = "line\n".repeat(50);
        let cut_down = cut(&long, 30);
        assert!(cut_down.ends_with("[...]\n"));
        assert!(cut_down.chars().count() <= 30, "{cut_down:?}");
        assert_eq!(cut(&long, 400), long);
    }

    #[test]
    fn tidy_keeps_one_blank_line() {
        assert_eq!(tidy("a\n\n\n\nb\n"), "a\n\nb");
        assert_eq!(tidy("   \n\nx"), "x");
        assert_eq!(tidy(""), "");
    }

    #[test]
    fn relative_links_resolve_against_the_page() -> Outcome {
        let base = site("https://example.com/docs/start")?;
        let resolved: Vec<String> = [
            "/a",
            "b",
            "../up",
            "//example.com/e",
            "https://example.com/f",
        ]
        .into_iter()
        .filter_map(|raw| base.absolute(raw))
        .map(|url| url.as_string())
        .collect();
        assert_eq!(
            resolved,
            [
                "https://example.com/a",
                "https://example.com/docs/b",
                "https://example.com/up",
                "https://example.com/e",
                "https://example.com/f",
            ]
        );
        assert_eq!(
            base.absolute("../c").map(|url| url.as_string()),
            Some("https://example.com/c".to_string())
        );
        Ok(())
    }
}
