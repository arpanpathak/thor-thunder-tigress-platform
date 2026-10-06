<!-- source: ripgrep/crates/grep/examples/simplegrep.rs; section: 0644daf9e8ea0492; licence: MIT -->
### User
I'm reading ripgrep's `simplegrep.rs` example. How do the matcher, searcher, printer and sink fit together?

### Assistant
The `grep` crate splits a search into independent parts, and the example wires one of each:

- **Matcher** (`RegexMatcher::new_line_matcher(pattern)`): knows only how to find the pattern in bytes. "Line matcher" sets the line terminator to `\n` so a match never spans two lines, which allows optimizations for line-oriented search: a pattern containing a literal `\n` is refused with an error, and `\n` is quietly removed from classes such as `\s`.
- **Searcher** (`SearcherBuilder`): reads the input (a file, here via `search_path`), splits it into lines, asks the matcher about them, and handles the reading concerns: binary detection (`BinaryDetection::quit(b'\x00')` stops at the first NUL byte, so binary files are skipped), line numbers, memory maps, encodings.
- **Printer** (`StandardBuilder`): formats results the way ripgrep's default output looks, with colors only when stdout is a terminal (`is_terminal()` picks `ColorChoice::Auto` or `Never`).
- **Sink** (`printer.sink_with_path(&matcher, path)`): the bridge. The searcher pushes each match into a sink, and this sink prints it with the file name. Implement your own `Sink` to collect matches into a `Vec` instead of printing.

Because the parts only meet through traits, you can swap any one: a different matcher (PCRE2), a searcher configured for multi-line search, a JSON printer.

The rest of the example is plumbing. `walkdir` lists the files under each path, errors for single entries are printed and skipped (`continue`) so one unreadable file does not stop the search, and `try_main` returns `Result<(), Box<dyn Error>>` so `main` has one place that prints the error and exits with status 1. Arguments are read with `args_os`, not `args`, because file paths need not be valid UTF-8.

### User
Without any crates, what would the core of a line search look like?

### Assistant
The part the searcher and matcher do for one file, for a literal pattern and text already in memory, fits in one function. It does none of the hard parts: no regex, no binary detection, no large files read in chunks.

```rust
/// One line containing the pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match<'a> {
    /// The line number, from 1.
    pub line_number: usize,
    /// The line, without its terminator.
    pub line: &'a str,
}

/// Every line of `text` that contains `pattern`. An empty pattern matches every line.
#[must_use]
pub fn search<'a>(pattern: &str, text: &'a str) -> Vec<Match<'a>> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains(pattern))
        .map(|(index, line)| Match { line_number: index + 1, line })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_lines_with_their_numbers() {
        let text = "fn main() {\n    println!(\"hello\");\n}\n// hello again\n";
        let found = search("hello", text);
        assert_eq!(found, [Match { line_number: 2, line: "    println!(\"hello\");" }, Match { line_number: 4, line: "// hello again" }]);
    }

    #[test]
    fn handles_crlf_and_no_matches() {
        assert_eq!(search("b", "a\r\nb\r\n"), [Match { line_number: 2, line: "b" }]);
        assert_eq!(search("zzz", "a\nb"), []);
    }
}
```

The matches borrow from `text` (the `'a` lifetime), so nothing is copied. `lines()` strips both `\n` and `\r\n`. Taking `&str` means the input must be valid UTF-8; ripgrep searches raw bytes (`&[u8]`) precisely so that files with invalid UTF-8 can still be searched.
