use super::{
    markdown::{strip_front_matter, strip_html},
    *,
};

#[test]
fn removes_heading_anchors() {
    assert_eq!(
        clean("# Processes and the Kernel {#processes}\n\nText.\n"),
        "# Processes and the Kernel\n\nText."
    );
}

#[test]
fn a_page_without_a_heading_takes_its_front_matter_title() {
    let cleaned =
        clean("---\ntitle: \"Pod Lifecycle\"\nweight: 30\n---\nPods follow a lifecycle.\n");

    assert!(
        cleaned.starts_with("# Pod Lifecycle\n\nPods follow"),
        "{cleaned}"
    );
}

#[test]
fn tidies_book_prose_but_leaves_code_alone() {
    let markdown = "Use raw identifiers, as in the [\u{201c}Raw\nIdentifiers\u{201d}][raw]<!-- ignore --> section.\n\n[raw]: #raw\n\n- `as`: casting, or rename\n  items in `use`.\n\n```rust\nlet x = grid[i][j]; // [a]: b\n```\n";
    let cleaned = clean(markdown);

    assert!(
        cleaned.contains("as in the \u{201c}Raw Identifiers\u{201d} section."),
        "{cleaned}"
    );
    assert!(!cleaned.contains("[raw]: #raw"));
    assert!(!cleaned.contains("ignore"));
    assert!(cleaned.contains("- `as`: casting, or rename items in `use`."));
    assert!(cleaned.contains("let x = grid[i][j]; // [a]: b"));
}

#[test]
fn caps_a_source_by_tokens_and_keeps_the_original_order() {
    let section = |text: &str| Example {
        instruction: String::new(),
        response: text.repeat(40),
        source: Source::Corpus,
        origin: String::new(),
    };
    let all: Vec<Example> = ["a", "b", "c", "d"]
        .iter()
        .map(|text| section(text))
        .collect();
    let kept = cap_tokens(all.clone(), 20);

    assert_eq!(kept.len(), 2);

    let positions: Vec<usize> = kept
        .iter()
        .filter_map(|example| all.iter().position(|candidate| candidate == example))
        .collect();

    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(cap_tokens(all.clone(), 1_000).len(), 4);
}

#[test]
fn classifies_the_licences_actually_fetched() {
    assert_eq!(
        Licence::classify("Apache License Version 2.0, January 2004"),
        Licence::Apache
    );
    assert_eq!(
        Licence::classify("Copyright (c) 2015 Aria Desires Permission is hereby granted"),
        Licence::Mit
    );
    assert_eq!(
        Licence::classify(
            "Copyright 2009 The Go Authors. Redistribution and use in source and binary forms"
        ),
        Licence::Bsd
    );
    assert_eq!(
        Licence::classify("Attribution 4.0 International"),
        Licence::CreativeCommons
    );
    assert_eq!(
        Licence::classify("Mozilla Public License Version 2.0"),
        Licence::MozillaPublic
    );
    assert_eq!(
        Licence::classify(
            "A. HISTORY OF THE SOFTWARE PYTHON SOFTWARE FOUNDATION LICENSE VERSION 2"
        ),
        Licence::PythonSoftwareFoundation
    );
    assert!(Licence::PythonSoftwareFoundation.permits_reuse());

    let personal_use_only = "Standard C++ Foundation grants you a worldwide, nonexclusive, royalty-free, perpetual license to copy, use, modify, and create derivative works from this project for your personal or internal business use only.";

    assert!(!Licence::classify(personal_use_only).permits_reuse());
    assert_eq!(
        Licence::classify("Attribution-NonCommercial-NoDerivatives 4.0 International"),
        Licence::CreativeCommonsRestricted
    );
    assert_eq!(
        Licence::classify("Creative Commons Legal Code Attribution-ShareAlike 3.0 Unported"),
        Licence::CreativeCommonsRestricted
    );
    assert_eq!(
        Licence::classify("Creative Commons Legal Code CC0 1.0 Universal"),
        Licence::CreativeCommons
    );
    assert_eq!(
        Licence::classify("CC0 1.0 Universal Statement of Purpose The laws of most jurisdictions"),
        Licence::CreativeCommons
    );
    assert_eq!(
        Licence::classify(
            "MIT License Permission is hereby granted || Attribution-NonCommercial 4.0 International"
        ),
        Licence::CreativeCommonsRestricted
    );
}

#[test]
fn mozilla_public_is_refused() {
    assert!(Licence::Mit.permits_reuse());
    assert!(Licence::Apache.permits_reuse());
    assert!(!Licence::MozillaPublic.permits_reuse());
    assert!(!Licence::Missing.permits_reuse());
    assert!(!Licence::Unrecognised.permits_reuse());
}

struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new(name: &str) -> Self {
        Self {
            root: std::env::temp_dir()
                .join(format!("thor-hammer-corpus-{name}-{}", std::process::id())),
        }
    }

    fn write(&self, path: &str, text: &str) -> Result<(), DataError> {
        let file = self.root.join(path);
        let parent = file.parent().unwrap_or(&self.root);
        fs::create_dir_all(parent).map_err(DataError::io(parent))?;

        fs::write(&file, text).map_err(DataError::io(&file))
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn walks_books_plain_sources_and_skips_what_it_should() -> Result<(), DataError> {
    let tree = Tree::new("walk");
    let section =
        "# Title\n\n## Part\n\nEnough words to make a section that is kept as an example.\n";

    tree.write("corpus/mybook/guide/book.toml", "[book]\nsrc = \"text\"\n")?;
    tree.write("corpus/mybook/guide/text/ch01.md", section)?;
    tree.write("corpus/mybook/guide/text/SUMMARY.md", section)?;
    tree.write("corpus/mybook/.hidden/book.toml", "")?;
    tree.write("corpus/mybook/a/b/c/d/e/book.toml", "")?;
    tree.write("corpus/aosa-500lines/ok/chapter.markdown", section)?;
    tree.write("corpus/aosa-500lines/incomplete/draft.md", section)?;
    tree.write("corpus/aosa-500lines/.git/x.md", section)?;
    tree.write("corpus/eng-practices/other/x.md", section)?;

    let manifest = "source\tkind\tcommit\tlicence_file\tlicence\nmybook\tbook\tabc\tLICENSE\tMIT License\naosa-500lines\tbook\tabc\tLICENSE\tMIT License\neng-practices\tdocs\tabc\tLICENSE\tMIT License\nnot-fetched\tbook\tabc\tLICENSE\tMIT License\nbroken line\n";
    tree.write("manifest.tsv", manifest)?;

    let corpus = examples(&tree.root.join("corpus"), &tree.root.join("manifest.tsv"))?;
    let origins: Vec<&str> = corpus
        .examples
        .iter()
        .map(|example| example.origin.as_str())
        .collect();

    assert_eq!(
        origins,
        [
            "mybook/guide/text/ch01.md",
            "aosa-500lines/ok/chapter.markdown"
        ]
    );

    let outcomes: Vec<(&str, bool)> = corpus
        .reports
        .iter()
        .map(|report| {
            (
                report.name.as_str(),
                matches!(report.outcome, SourceOutcome::NoMarkdown),
            )
        })
        .collect();

    assert_eq!(
        outcomes,
        [
            ("mybook", false),
            ("aosa-500lines", false),
            ("eng-practices", true),
            ("not-fetched", true)
        ]
    );

    Ok(())
}

#[test]
fn names_every_licence() {
    let names: Vec<&str> = [
        Licence::Mit,
        Licence::Apache,
        Licence::Bsd,
        Licence::CreativeCommons,
        Licence::CreativeCommonsRestricted,
        Licence::MozillaPublic,
        Licence::PythonSoftwareFoundation,
        Licence::Unrecognised,
        Licence::Missing,
    ]
    .into_iter()
    .map(Licence::name)
    .collect();

    assert_eq!(
        names,
        [
            "MIT",
            "Apache-2.0",
            "BSD",
            "CC-BY",
            "CC with NC/ND/SA terms",
            "MPL-2.0",
            "PSF-2.0",
            "unrecognised",
            "missing"
        ]
    );
}

#[test]
fn keeps_an_unclosed_tag_and_an_unclosed_front_matter() {
    assert_eq!(strip_html("a < b"), "a < b");
    assert_eq!(
        strip_front_matter("---\ntitle: x\nno end"),
        "---\ntitle: x\nno end"
    );
}

#[test]
fn reads_the_source_directory_from_book_toml() {
    assert_eq!(book_source_in("[book]\nsrc = \"src\"\n"), "src");
    assert_eq!(
        book_source_in("[book]\ntitle = \"x\"\n"),
        DEFAULT_BOOK_SOURCE
    );
    assert_eq!(book_source_in("[book]\nsrc = 'book-src'\n"), "book-src");
}

#[test]
fn removes_front_matter_and_shortcodes() {
    let markdown = "---\ntitle: x\n---\n\n# Head\n\n{{< note >}}\n\nText.\n";
    let cleaned = clean(markdown);

    assert!(cleaned.starts_with("\n# Head"));
    assert!(!cleaned.contains("title: x"));
    assert!(!cleaned.contains("{{"));
    assert!(cleaned.contains("Text."));
}

#[test]
fn keeps_markdown_without_front_matter() {
    assert_eq!(clean("# Head\n\nText.\n"), "# Head\n\nText.");
}

#[test]
fn removes_source_markup_but_keeps_the_text() {
    assert_eq!(
        strip_html("<span class=\"caption\">Table B-1: Operators</span>"),
        "Table B-1: Operators"
    );
    assert_eq!(strip_html("a<br>b"), "a b");
}

#[test]
fn keeps_rust_generics() {
    assert_eq!(
        strip_html("a Vec<i32> and a HashMap<K, V>"),
        "a Vec<i32> and a HashMap<K, V>"
    );
    assert_eq!(strip_html("`Result<T, E>`"), "`Result<T, E>`");
}

#[test]
fn decodes_entities() {
    assert_eq!(strip_html("a &amp; b"), "a & b");
    assert_eq!(strip_html("x &lt; y"), "x < y");
}
