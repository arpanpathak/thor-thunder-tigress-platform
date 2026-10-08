//! Examples from source files that open with a comment describing them.
//!
//! ## Design
//!
//! The header comment says what the file does, so it becomes the request and
//! the whole file becomes the answer:
//!
//! ```text
//!   //! Top K frequent elements.      ─► "Write a Rust program for this: Top K frequent elements."
//!   use std::collections::...         ─► the file, in a ```rust block
//! ```
//!
//! A file without a header comment gives no example, because there is nothing
//! to turn into a request.

use std::path::Path;

use crate::example::{Example, Source};

/// How one language marks the comment that describes a file.
struct Language {
    /// The file extension, without the dot.
    extension: &'static str,
    /// The name used in the request.
    name: &'static str,
    /// The tag on the markdown code block in the answer.
    code_block_tag: &'static str,
    /// The comment marker of the header lines.
    header_marker: &'static str,
}

/// The languages in the book repository.
const LANGUAGES: [Language; 4] = [
    Language {
        extension: "rs",
        name: "Rust",
        code_block_tag: "rust",
        header_marker: "//!",
    },
    Language {
        extension: "go",
        name: "Go",
        code_block_tag: "go",
        header_marker: "//",
    },
    Language {
        extension: "cu",
        name: "CUDA",
        code_block_tag: "cuda",
        header_marker: "//",
    },
    Language {
        extension: "py",
        name: "Python",
        code_block_tag: "python",
        header_marker: "#",
    },
];

/// True for the languages listed in [`LANGUAGES`].
pub fn is_source_file(path: &Path) -> bool {
    language(path).is_some()
}

/// The example for one source file, or `None` when it has no header comment.
pub fn example(source_code: &str, path: &Path, origin: &str) -> Option<Example> {
    let language = language(path)?;
    let header = header_comment(source_code, language.header_marker)?;
    Some(Example {
        instruction: format!("Write a {} program for this:\n\n{header}", language.name),
        response: format!(
            "```{}\n{}\n```",
            language.code_block_tag,
            source_code.trim_end()
        ),
        source: Source::Code,
        origin: origin.to_string(),
    })
}

/// The language of `path`, found by its extension.
fn language(path: &Path) -> Option<&'static Language> {
    let extension = path.extension()?.to_str()?;
    LANGUAGES
        .iter()
        .find(|language| language.extension == extension)
}

/// The comment lines at the top of the file, without their markers. Blank lines
/// and a `#!` interpreter line before the comment are skipped. `None` when the
/// file does not start with a comment.
fn header_comment(source_code: &str, header_marker: &str) -> Option<String> {
    let header_lines: Vec<&str> = source_code
        .lines()
        .map(str::trim)
        .skip_while(|line| line.is_empty() || line.starts_with("#!"))
        .map_while(|line| line.strip_prefix(header_marker))
        .map(str::trim)
        .collect();
    let header = header_lines.join("\n").trim().to_string();

    match header.is_empty() {
        true => None,
        false => Some(header),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction(source_code: &str, file_name: &str) -> Option<String> {
        example(source_code, Path::new(file_name), file_name).map(|example| example.instruction)
    }

    #[test]
    fn rust_header_becomes_the_request() {
        let source_code = "//! Top K frequent elements.\n//!\n//! Uses a heap.\n\nuse std::collections::BinaryHeap;\n";
        let expected = "Write a Rust program for this:\n\nTop K frequent elements.\n\nUses a heap.";
        assert_eq!(
            instruction(source_code, "topk.rs").as_deref(),
            Some(expected)
        );
    }

    #[test]
    fn python_header_after_interpreter_line() {
        let source_code = "#!/usr/bin/env python3\n# Draw the page table figure.\nimport sys\n";
        let expected = "Write a Python program for this:\n\nDraw the page table figure.";
        assert_eq!(
            instruction(source_code, "draw.py").as_deref(),
            Some(expected)
        );
    }

    #[test]
    fn file_without_header_gives_no_example() {
        assert_eq!(instruction("use std::fs;\nfn main() {}\n", "a.rs"), None);
    }
}
