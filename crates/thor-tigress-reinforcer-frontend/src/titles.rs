//! Readable names for what the training file calls by folder or pipeline
//! name: sources, books, and the five rules.

use thor_spark_safety_eval::rules::Rule;

/// The name a reviewer sees for a source of the training file.
pub fn source_title(source: &str) -> String {
    match source {
        "readability" => "Your readability pairs".to_string(),
        "clever_vs_readable" => "Your clever vs readable pairs".to_string(),
        "chat" => "Claude chat export".to_string(),
        "book" => "Your interview books".to_string(),
        "code" => "Your code files".to_string(),
        "corpus" => "Open-source books and docs".to_string(),
        "conversation" => "Generated conversations".to_string(),
        "teacher" => "Teacher conversations".to_string(),
        other => humanize(other),
    }
}

/// The title of a book or document set, from the folder it was read from.
pub fn collection_title(folder: &str) -> String {
    let known = match folder {
        "trpl" => "The Rust Programming Language",
        "rbe" => "Rust by Example",
        "nomicon" => "The Rustonomicon",
        "async-book" => "Asynchronous Programming in Rust",
        "api-guidelines" => "Rust API Guidelines",
        "rustc-dev-guide" => "Rust Compiler Development Guide",
        "patterns" => "Rust Design Patterns",
        "too-many-lists" => "Learn Rust With Entirely Too Many Linked Lists",
        "comprehensive-rust" => "Comprehensive Rust (Google)",
        "reference" => "The Rust Reference",
        "cargo-book" => "The Cargo Book",
        "clippy" => "Clippy Lint Documentation",
        "google-styleguide" => "Google Style Guides",
        "ms-style-guide" => "Microsoft Writing Style Guide",
        "kubernetes-website" => "Kubernetes Documentation",
        "go-website" => "Go Documentation",
        "cracking-the-systems-programming-interview" => "Cracking the Systems Programming Interview",
        "cracking-the-systems-engineering-book" => "Cracking the Systems Engineering Book",
        "gpu-accelerated-kubernetes" => "GPU-Accelerated Kubernetes",
        "rust-interview-lab" => "Rust Interview Lab",
        "systems_design" => "Systems Design",
        "docs" => "Interview Prep Docs",
        _ => "",
    };
    match known {
        "" => humanize(folder),
        title => title.to_string(),
    }
}

/// What a broken rule means, in words.
pub fn rule_title(rule: Rule) -> &'static str {
    match rule {
        Rule::NoUnwrap => "Calls unwrap() or expect()",
        Rule::ErrorEnum => "Error type is not a custom enum",
        Rule::PubDocs => "Public item has no doc comment",
        Rule::NoBodyComments => "Comment inside a function body",
        Rule::NoIndexLoops => "Index loop instead of an iterator",
    }
}

/// `some-folder_name` as `Some folder name`.
fn humanize(name: &str) -> String {
    let spaced = name.replace(['-', '_'], " ");
    let mut characters = spaced.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_teacher_set_every_rule_and_an_empty_name() {
        use thor_spark_safety_eval::rules::Rule;
        assert_eq!(source_title("teacher"), "Teacher conversations");
        assert_eq!(source_title("field_notes"), "Field notes");
        assert!(Rule::ALL.iter().all(|&rule| !rule_title(rule).is_empty()));
        assert_eq!(humanize(""), "");
    }

    #[test]
    fn names_known_books_and_humanizes_the_rest() {
        assert_eq!(collection_title("trpl"), "The Rust Programming Language");
        assert_eq!(collection_title("new-book_draft"), "New book draft");
    }

    #[test]
    fn names_sources_in_words() {
        assert_eq!(source_title("corpus"), "Open-source books and docs");
    }
}
