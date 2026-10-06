//! Finds the comments in Rust source.
//!
//! `syn` throws ordinary comments away, so rule 4 needs its own pass. This is a
//! lexer, not a parser: it knows enough about string, raw string and character
//! literals that a `//` inside `"http://..."` is not taken for a comment.

use std::str::Chars;

/// One comment and where it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// The 1-based line, as `proc_macro2::LineColumn` counts it.
    pub line: usize,
    /// The 0-based column in characters, as `proc_macro2::LineColumn` counts it.
    pub column: usize,
    /// The comment, including its `//` or `/*`.
    pub text: String,
    /// True for `///`, `//!`, `/**` and `/*!`.
    pub doc: bool,
}

/// Walks the source one character at a time and keeps the position.
struct Cursor<'a> {
    rest: Chars<'a>,
    line: usize,
    column: usize,
    previous: char,
}

impl Cursor<'_> {
    fn bump(&mut self) -> Option<char> {
        let next = self.rest.next()?;
        match next {
            '\n' => {
                self.line += 1;
                self.column = 0;
            }
            _ => self.column += 1,
        }
        self.previous = next;
        Some(next)
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.rest.clone().nth(ahead)
    }

    fn line_comment(&mut self, line: usize, column: usize) -> Comment {
        let mut text = String::from("/");
        while let Some(next) = self.peek(0).filter(|next| *next != '\n') {
            text.push(next);
            self.bump();
        }
        let doc = (text.starts_with("///") && !text.starts_with("////")) || text.starts_with("//!");
        Comment { line, column, text, doc }
    }

    fn block_comment(&mut self, line: usize, column: usize) -> Comment {
        let mut text = String::from("/*");
        let mut depth = 1usize;
        self.bump();
        while let Some(next) = self.bump() {
            text.push(next);
            match (next, self.peek(0)) {
                ('/', Some('*')) => {
                    depth += 1;
                    text.push('*');
                    self.bump();
                }
                ('*', Some('/')) if depth <= 1 => {
                    text.push('/');
                    self.bump();
                    break;
                }
                ('*', Some('/')) => {
                    depth -= 1;
                    text.push('/');
                    self.bump();
                }
                _ => {}
            }
        }
        let doc = (text.starts_with("/**") && !text.starts_with("/***") && text != "/**/")
            || text.starts_with("/*!");
        Comment { line, column, text, doc }
    }

    fn skip_string(&mut self) {
        while let Some(next) = self.bump() {
            match next {
                '\\' => {
                    self.bump();
                }
                '"' => return,
                _ => {}
            }
        }
    }

    fn skip_raw_string(&mut self) {
        let mut hashes = 0usize;
        while self.peek(0) == Some('#') {
            hashes += 1;
            self.bump();
        }
        self.bump();
        while let Some(next) = self.bump() {
            let closes = next == '"' && (0..hashes).all(|ahead| self.peek(ahead) == Some('#'));
            if closes {
                for _ in 0..hashes {
                    self.bump();
                }
                return;
            }
        }
    }

    fn skip_char_literal(&mut self) {
        match (self.peek(0), self.peek(1)) {
            (Some('\\'), _) => {
                self.bump();
                self.bump();
                while let Some(next) = self.bump() {
                    if next == '\'' {
                        return;
                    }
                }
            }
            (Some(_), Some('\'')) => {
                self.bump();
                self.bump();
            }
            _ => {}
        }
    }

    fn starts_raw_string(&self, current: char, before: char) -> bool {
        let after_prefix = match current {
            'r' => 0,
            'b' if self.peek(0) == Some('r') => 1,
            _ => return false,
        };
        let hashes = self
            .rest
            .clone()
            .skip(after_prefix)
            .take_while(|next| *next == '#')
            .count();
        let fresh_token = !(before.is_alphanumeric() || before == '_');
        fresh_token && self.peek(after_prefix + hashes) == Some('"')
    }
}

/// Every comment in `source`, in order.
#[must_use]
pub fn find(source: &str) -> Vec<Comment> {
    let mut cursor = Cursor {
        rest: source.chars(),
        line: 1,
        column: 0,
        previous: ' ',
    };
    let mut comments = Vec::new();
    loop {
        let before = cursor.previous;
        let (line, column) = (cursor.line, cursor.column);
        let Some(current) = cursor.bump() else {
            return comments;
        };
        match (current, cursor.peek(0)) {
            ('/', Some('/')) => comments.push(cursor.line_comment(line, column)),
            ('/', Some('*')) => comments.push(cursor.block_comment(line, column)),
            ('"', _) => cursor.skip_string(),
            ('\'', _) => cursor.skip_char_literal(),
            (prefix, _) if cursor.starts_raw_string(prefix, before) => {
                if prefix == 'b' {
                    cursor.bump();
                }
                cursor.skip_raw_string();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(source: &str) -> Vec<String> {
        find(source)
            .into_iter()
            .map(|comment| comment.text)
            .collect()
    }

    #[test]
    fn finds_line_and_block_comments_with_positions() {
        let found = find("fn a() {\n    // why\n    /* how */ 1\n}");
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].column), (2, 4));
        assert_eq!(found[1].text, "/* how */");
    }

    #[test]
    fn ignores_slashes_inside_strings() {
        assert_eq!(texts(r#"let url = "http://x"; let raw = r"//"; let c = '/';"#), Vec::<String>::new());
    }

    #[test]
    fn ignores_slashes_inside_raw_strings_with_hashes() {
        assert_eq!(texts(r##"let s = r#"a "quoted" // part"#;"##), Vec::<String>::new());
    }

    #[test]
    fn keeps_lifetimes_from_swallowing_comments() {
        assert_eq!(texts("fn a<'a>(x: &'a str) {} // tail"), vec!["// tail"]);
    }

    #[test]
    fn marks_doc_comments() {
        let found = find("/// doc\n//! inner\n//// rule\n// plain");
        let docs: Vec<bool> = found.iter().map(|comment| comment.doc).collect();
        assert_eq!(docs, vec![true, true, false, false]);
    }

    #[test]
    fn handles_nested_block_comments() {
        assert_eq!(texts("/* a /* b */ c */ x"), vec!["/* a /* b */ c */"]);
    }
}
