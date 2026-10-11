use super::*;
use crate::teacher;

/// A fresh folder for one test, removed when dropped.
fn scratch() -> Result<tempfile::TempDir, DataError> {
    tempfile::tempdir().map_err(DataError::io(Path::new("temporary folder")))
}

fn conversation(answer: &str) -> Result<Conversation, DataError> {
    let entry = format!("<!-- source: test -->\n### User\nWrite it.\n### Assistant\n{answer}\n");

    teacher::parse(&entry, "test").map_err(DataError::format("test"))
}

const GOOD: &str = "Sums the values.\n\n```rust\n/// The sum of `values`.\n#[must_use]\npub fn total(values: &[u32]) -> u32 {\n    values.iter().sum()\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn adds() {\n        assert_eq!(total(&[1, 2]), 3);\n    }\n}\n```";

#[test]
fn a_clean_block_builds_and_its_tests_count() -> Result<(), DataError> {
    let folder = scratch()?;
    let checked = check(&conversation(GOOD)?, folder.path())?;

    assert_eq!(
        (checked.block_count(), checked.tests, checked.problems.len()),
        (1, 1, 0)
    );

    Ok(())
}

fn kinds(checked: &Checked) -> Vec<&'static str> {
    checked.problems.iter().map(Problem::kind).collect()
}

#[test]
fn a_failing_test_is_a_problem() -> Result<(), DataError> {
    let folder = scratch()?;
    let checked = check(&conversation(&GOOD.replace("3);", "4);"))?, folder.path())?;

    assert_eq!(kinds(&checked), ["tests"]);

    Ok(())
}

#[test]
fn an_unwrap_breaks_a_rule_and_clippy_pedantic() -> Result<(), DataError> {
    let folder = scratch()?;
    let source = GOOD.replace("values.iter().sum()", "values.first().copied().unwrap()");
    let checked = check(&conversation(&source)?, folder.path())?;

    assert_eq!(kinds(&checked), ["rule", "build"]);

    Ok(())
}

#[test]
fn a_clippy_warning_fails_the_build() -> Result<(), DataError> {
    let folder = scratch()?;
    let linted = GOOD.replace(
        "values.iter().sum()",
        "values.iter().fold(0, |sum, value| sum + value)",
    );
    let checked = check(&conversation(&linted)?, folder.path())?;

    assert!(matches!(
        checked.problems.as_slice(),
        [Problem::Build {
            turn: 2,
            language: "rust",
            ..
        }]
    ));

    Ok(())
}

#[test]
fn slop_and_false_claims_are_problems() -> Result<(), DataError> {
    let folder = scratch()?;
    let sloppy = "Great question! This code never uses unwrap.\n\n```rust\n/// One.\n#[must_use]\npub fn one(values: &[u8]) -> u8 {\n    values.first().copied().unwrap()\n}\n```";
    let checked = check(&conversation(sloppy)?, folder.path())?;

    assert!(
        checked
            .problems
            .iter()
            .any(|problem| matches!(problem, Problem::Slop { .. }))
    );
    assert!(
        checked
            .problems
            .iter()
            .any(|problem| matches!(problem, Problem::FalseClaim { .. }))
    );
    assert!(!spark_objections(sloppy).is_empty());

    Ok(())
}

#[test]
fn only_a_main_function_makes_a_program() {
    assert_eq!(CrateType::of("fn main() {}"), CrateType::Bin);
    assert_eq!(
        CrateType::of("const CODE: &str = \"fn main() {}\";"),
        CrateType::Lib
    );
}

#[test]
fn checks_other_languages_and_counts_them() -> Result<(), DataError> {
    let folder = scratch()?;
    let answer =
        "Two blocks.\n\n```python\nassert max([3, 9]) == 9\n```\n\n```python\ndef f(:\n```";
    let checked = check(&conversation(answer)?, folder.path())?;

    assert_eq!(checked.blocks.get("python"), Some(&2));
    assert_eq!(checked.runs, 1);
    assert!(matches!(
        checked.problems.as_slice(),
        [Problem::Build {
            language: "python",
            ..
        }]
    ));

    Ok(())
}

#[test]
fn a_grounded_entry_keeps_broken_rules_as_notes() -> Result<(), DataError> {
    let folder = scratch()?;
    let answer = "```rust\n/// One.\n#[must_use]\npub fn one(text: &str) -> u8 {\n    text.parse().unwrap_or(1)\n}\n\nfn main() {\n    let n: u8 = \"1\".parse().unwrap();\n    println!(\"{}\", one(\"x\") + n);\n}\n```";
    let entry = format!(
        "<!-- source: trpl; section: abc -->\n### User\nShow unwrap.\n### Assistant\n{answer}\n"
    );
    let grounded = teacher::parse(&entry, "x").map_err(DataError::format("x"))?;
    let checked = check(&grounded, folder.path())?;

    assert!(checked.problems.is_empty(), "{:?}", checked.problems);
    assert_eq!(checked.notes.len(), 1);

    Ok(())
}

#[test]
fn a_command_over_its_limit_is_killed() -> Result<(), DataError> {
    let folder = scratch()?;
    fs::create_dir_all(folder.path()).map_err(DataError::io(folder.path()))?;

    let mut sleeper = Command::new("sleep");
    sleeper.arg("5");

    assert_eq!(
        run_limited(sleeper, folder.path(), Duration::from_millis(50))?,
        Finished::OverTime
    );
    assert_eq!(Built::after_run(Finished::OverTime), Built::TimedOut);
    assert_eq!(
        Built::after_run(Finished::Failed("x".to_string())),
        Built::TestsFailed("x".to_string())
    );

    Ok(())
}

#[test]
fn records_every_outcome_of_a_block() {
    let mut checked = Checked::default();
    checked.record(2, "rust", Built::Ignored);
    checked.record(2, "go", Built::TimedOut);
    checked.record(2, "go", Built::TestsFailed("panic".to_string()));
    checked.record(4, "python", Built::BuildFailed("syntax".to_string()));

    assert_eq!(checked.ignored, 1);
    assert_eq!(checked.block_count(), 3);

    let lines: Vec<String> = checked.problems.iter().map(ToString::to_string).collect();

    assert_eq!(
        lines,
        [
            "turn 2: tests ran over 30 s",
            "turn 2: go tests or program fail\npanic",
            "turn 4: python does not build cleanly\nsyntax"
        ]
    );
}

#[test]
fn a_warning_only_in_test_code_fails_the_test_build() -> Result<(), DataError> {
    let folder = scratch()?;
    let linted = GOOD.replace(
        "assert_eq!(total(&[1, 2]), 3);",
        "let unused = 1;\n        assert_eq!(total(&[1, 2]), 3);",
    );
    let checked = check(&conversation(&linted)?, folder.path())?;

    assert_eq!(kinds(&checked), ["build"]);

    Ok(())
}

#[test]
fn names_every_kind_of_problem() {
    let problems = [
        Problem::Build {
            turn: 1,
            language: "rust",
            output: String::new(),
        },
        Problem::Tests {
            turn: 1,
            language: "rust",
            output: String::new(),
        },
        Problem::TimedOut { turn: 1 },
        Problem::Rule {
            turn: 1,
            detail: String::new(),
        },
        Problem::Slop {
            turn: 1,
            phrases: Vec::new(),
        },
        Problem::FalseClaim {
            turn: 1,
            text: String::new(),
        },
    ];
    let kinds: Vec<&str> = problems.iter().map(Problem::kind).collect();

    assert_eq!(
        kinds,
        ["build", "tests", "timed out", "rule", "slop", "false claim"]
    );
}

#[test]
fn a_block_that_does_not_parse_and_em_dashes_are_problems() -> Result<(), DataError> {
    let folder = scratch()?;
    let answer = "One \u{2014} two \u{2014} three.\n\n```rust,ignore\nfn (\n```";
    let kinds_found = kinds(&check(&conversation(answer)?, folder.path())?);

    assert_eq!(kinds_found, ["rule", "slop"]);

    Ok(())
}

#[test]
fn counts_passed_tests_across_result_lines() {
    assert_eq!(
        passed_tests("test result: ok. 3 passed; 0 failed\ntest result: ok. 2 passed;"),
        5
    );
    assert_eq!(passed_tests("nothing"), 0);
}

#[test]
fn describes_each_problem_with_its_turn() {
    let problems = [
        Problem::TimedOut { turn: 2 },
        Problem::Slop {
            turn: 4,
            phrases: vec!["Great question".to_string()],
        },
        Problem::FalseClaim {
            turn: 2,
            text: "never unwraps".to_string(),
        },
    ];
    let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();

    assert_eq!(
        lines,
        [
            "turn 2: tests ran over 30 s",
            "turn 4: slop: Great question",
            "turn 2: false claim: never unwraps"
        ]
    );
}
