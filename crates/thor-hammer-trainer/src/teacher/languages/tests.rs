use super::*;

/// A fresh folder for one test, removed when dropped.
fn scratch() -> Result<tempfile::TempDir, DataError> {
    tempfile::tempdir().map_err(DataError::io(Path::new("temporary folder")))
}

/// What a block that builds and runs cleanly, without tests, is found to be.
const RAN: Built = Built::Clean {
    tests: 0,
    ran: true,
};

/// What a block that only parses or compiles is found to be.
const PARSED: Built = Built::Clean {
    tests: 0,
    ran: false,
};

fn checked(language: Language, code: &str) -> Result<Built, DataError> {
    let folder = scratch()?;
    let block = Block {
        language,
        code: code.to_string(),
        ignored: false,
    };
    let built = check(&block, folder.path())?;

    Ok(built)
}

#[test]
fn finds_tagged_blocks_only() {
    let text = "```python\nprint(1)\n```\n\n```\nuntagged\n```\n\n```text\nplain\n```\n\n```Go\npackage main\n```\n";
    let found: Vec<Language> = blocks(text)
        .into_iter()
        .map(|block| block.language)
        .collect();

    assert_eq!(found, [Language::Python, Language::Go]);
}

#[test]
fn an_ignored_fence_is_not_built() -> Result<(), DataError> {
    let text = "```go,ignore\nfmt.Println(x)\n```\n\n```rust,no_run\nfn main() {}\n```\n";
    let found = fenced(text);

    assert_eq!(
        found
            .iter()
            .map(|block| (block.tag.as_str(), block.ignored))
            .collect::<Vec<_>>(),
        [("go", true), ("rust", false)]
    );

    let ignored = blocks(text)
        .into_iter()
        .next()
        .map(|block| check(&block, Path::new("never written")));

    assert!(matches!(ignored, Some(Ok(Built::Ignored))));

    Ok(())
}

#[test]
fn runs_a_python_program_and_catches_a_failed_assert() -> Result<(), DataError> {
    assert_eq!(checked(Language::Python, "assert sum([1, 2]) == 3\n")?, RAN);
    assert!(matches!(
        checked(Language::Python, "assert 1 == 2\n")?,
        Built::TestsFailed(_)
    ));
    assert!(matches!(
        checked(Language::Python, "def f(:\n")?,
        Built::BuildFailed(_)
    ));

    Ok(())
}

#[test]
fn builds_go_cpp_and_java() -> Result<(), DataError> {
    let go = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n";

    assert_eq!(checked(Language::Go, go)?, RAN);

    let cpp =
        "#include <vector>\nint main() { std::vector<int> v{1}; return v.size() == 1 ? 0 : 1; }\n";

    assert_eq!(checked(Language::Cpp, cpp)?, RAN);

    let java = "public class Hello {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n";

    assert_eq!(checked(Language::Java, java)?, RAN);
    assert_eq!(
        checked(Language::Cpp, "inline int twice(int x) { return 2 * x; }\n")?,
        PARSED
    );

    Ok(())
}

#[test]
fn parses_data_and_scripts_without_running_them() -> Result<(), DataError> {
    assert_eq!(checked(Language::Yaml, "a: [1, 2]\n")?, PARSED);
    assert!(matches!(
        checked(Language::Yaml, "a: [1, 2\n")?,
        Built::BuildFailed(_)
    ));
    assert!(matches!(
        checked(Language::Json, "{\"a\": }")?,
        Built::BuildFailed(_)
    ));
    assert_eq!(checked(Language::Shell, "set -eu\necho hi\n")?, PARSED);
    assert!(matches!(
        checked(Language::JavaScript, "const a = ;\n")?,
        Built::BuildFailed(_)
    ));

    let tested = "const assert = require(\"node:assert\");\nassert.strictEqual(1 + 1, 2);\n";

    assert_eq!(checked(Language::JavaScript, tested)?, RAN);

    Ok(())
}

#[test]
fn names_every_language() {
    let names: Vec<&str> = ["py", "go", "c++", "java", "js", "sh", "yml", "json"]
        .into_iter()
        .filter_map(Language::of)
        .map(Language::name)
        .collect();

    assert_eq!(
        names,
        [
            "python",
            "go",
            "c++",
            "java",
            "javascript",
            "bash",
            "yaml",
            "json"
        ]
    );
    assert_eq!(Language::of("toml"), None);
}
