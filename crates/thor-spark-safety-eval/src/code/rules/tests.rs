use super::*;

const UNCOMMON_FORMS: &str = r##"use std::{fmt, io};

/// A struct used as an error.
#[derive(Debug)]
pub struct Failure;

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("failure")
    }
}

impl std::error::Error for Failure {}

#[derive(Debug, thiserror::Error)]
pub enum Derived {}

pub trait Shape {
    fn area(&self) -> f64 {
        "1".parse().unwrap()
    }
}

pub type Alias = u8;
pub const LIMIT: u8 = 1;
pub static NAME: &str = "x";
pub mod inner {}
pub union Bits { a: u8 }

pub fn text() -> Result<(), &'static str> { Err("x") }
pub fn borrowed() -> Result<(), &Failure> { Ok(()) }
pub fn slice() -> Result<(), &[u8]> { Ok(()) }
pub fn tuple() -> Result<(), (u8, u8)> { Ok(()) }
pub fn aliased() -> Result<u8> { Ok(1) }
pub fn io_result() -> io::Result<u8> { Ok(1) }
pub fn array() -> [u8; 2] { [1, 2] }
pub fn bare() -> Result { todo!() }
pub fn three() -> Result<u8, u8, u8> { todo!() }
pub fn boxed() -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
pub fn owned() -> Result<(), String> { Ok(()) }

pub fn lifetimes<'a>() -> Result<'a, u8, Failure> { todo!() }

use anyhow;
use anyhow as ah;
use {anyhow::Context};

pub fn macros() -> anyhow::Result<()> {
    println!("{}", Some(1).unwrap());
    let items = vec![Some(2).unwrap()];
    anyhow::bail!("stop {}", items.len());
}
"##;

#[test]
fn finds_an_index_inside_a_macro_and_shortens_a_long_comment() {
    let source = "/// Prints.\npub fn show(values: &[u8]) {\n    for i in 0..values.len() {\n        println!(\"{}\", values[i]);\n    }\n    // This comment inside the body is much longer than sixty characters in total.\n}\n";
    let details: Vec<String> = check(source)
        .violations
        .into_iter()
        .map(|violation| violation.detail)
        .collect();

    assert!(
        details
            .iter()
            .any(|detail| detail == "for i in a range, used as an index"),
        "{details:?}"
    );

    let unnamed = check(
        "/// Counts.\npub fn count(values: &[u8]) {\n    for _ in 0..values.len() {}\n    for value in values {\n        drop(value);\n    }\n}\n",
    );

    assert!(
        unnamed
            .violations
            .iter()
            .any(|violation| violation.detail == "for _ in a range, used as an index")
    );
    assert!(
        details
            .iter()
            .any(|detail| detail.ends_with('…') && detail.chars().count() == DETAIL_CHARS + 1),
        "{details:?}"
    );
}

#[test]
fn reports_every_uncommon_form() {
    let report = check(UNCOMMON_FORMS);
    let found: Vec<(usize, &str)> = report
        .violations
        .iter()
        .map(|violation| (violation.line, violation.detail.as_str()))
        .collect();
    let expected = [
        (13, "Failure is a struct, not an enum"),
        (20, ".unwrap()"),
        (27, "pub mod inner has no doc comment"),
        (28, "pub union Bits has no doc comment"),
        (30, "returns Result<_, &str>"),
        (31, "Failure is a struct, not an enum"),
        (35, "returns the library error io::Error, not a custom enum"),
        (39, "returns Result<_, Box<dyn Error>>"),
        (40, "returns Result<_, String>"),
        (49, ".unwrap()"),
        (50, ".unwrap()"),
    ];

    assert!(
        expected.iter().all(|wanted| found.contains(wanted)),
        "{found:?}"
    );

    let undocumented = found
        .iter()
        .filter(|(_, detail)| detail.ends_with("has no doc comment"))
        .count();

    assert_eq!(undocumented, 20);
    assert_eq!(report.violations.len(), 29);
}

fn verdict(source: &str, rule: Rule) -> Verdict {
    check(source)
        .verdicts
        .get(&rule)
        .copied()
        .unwrap_or(Verdict::NotApplicable)
}

#[test]
fn finds_unwrap_and_expect_including_inside_macros() {
    let report = check("fn a() { let x = b().unwrap(); assert_eq!(c().expect(\"c\"), 1); }");
    let found: Vec<&str> = report
        .violations
        .iter()
        .filter(|violation| violation.rule == Rule::NoUnwrap)
        .map(|violation| violation.detail.as_str())
        .collect();

    assert_eq!(found, vec![".unwrap()", ".expect()"]);
}

#[test]
fn unwrap_or_is_allowed() {
    assert_eq!(
        verdict("fn a() { b().unwrap_or(0); }", Rule::NoUnwrap),
        Verdict::Pass
    );
}

#[test]
fn rule_two_does_not_apply_to_code_without_error_handling() {
    assert_eq!(
        verdict("fn add(a: i32, b: i32) -> i32 { a + b }", Rule::ErrorEnum),
        Verdict::NotApplicable
    );
}

#[test]
fn rule_two_fails_string_errors_and_boxed_errors() {
    assert_eq!(
        verdict("fn a() -> Result<(), String> { Ok(()) }", Rule::ErrorEnum),
        Verdict::Fail
    );
    assert_eq!(
        verdict(
            "fn main() -> Result<(), Box<dyn std::error::Error>> { Ok(()) }",
            Rule::ErrorEnum
        ),
        Verdict::Fail
    );
}

#[test]
fn rule_two_passes_anyhow_and_an_enum_derived_with_thiserror() {
    let derived = "
            #[derive(Debug, thiserror::Error)] enum E { #[error(\"bad\")] Bad }
            fn a() -> Result<(), E> { Err(E::Bad) }
        ";
    let derived_struct = "
            #[derive(Debug, thiserror::Error)] #[error(\"bad\")] struct E;
            fn a() -> Result<(), E> { Err(E) }
        ";
    let bailing = "fn a() -> anyhow::Result<()> { anyhow::bail!(\"stop\") }";

    assert_eq!(verdict(derived, Rule::ErrorEnum), Verdict::Pass);
    assert_eq!(verdict(derived_struct, Rule::ErrorEnum), Verdict::Fail);
    assert_eq!(
        verdict("use anyhow::Result;", Rule::ErrorEnum),
        Verdict::Pass
    );
    assert_eq!(verdict(bailing, Rule::ErrorEnum), Verdict::Pass);
}

#[test]
fn rule_two_passes_a_hand_written_enum() {
    let source = "
            #[derive(Debug)] enum E { Bad }
            impl std::fmt::Display for E { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(\"bad\") } }
            impl std::error::Error for E {}
            fn a() -> Result<(), E> { Err(E::Bad) }
        ";

    assert_eq!(verdict(source, Rule::ErrorEnum), Verdict::Pass);
}

#[test]
fn rule_two_does_not_judge_a_signature_a_trait_imposes() {
    let source = "struct R; impl std::io::Read for R { fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> { Ok(0) } }";

    assert_eq!(verdict(source, Rule::ErrorEnum), Verdict::NotApplicable);
}

#[test]
fn rule_two_follows_a_renamed_error_trait() {
    let source = "
            use std::error::Error as StdError;
            enum E { Bad }
            impl std::fmt::Display for E { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(\"bad\") } }
            impl StdError for E {}
            fn a() -> Result<(), E> { Err(E::Bad) }
        ";

    assert_eq!(verdict(source, Rule::ErrorEnum), Verdict::Pass);
}

#[test]
fn rule_two_fails_an_enum_without_error_impl() {
    let source = "enum E { Bad } fn a() -> Result<(), E> { Err(E::Bad) }";

    assert_eq!(verdict(source, Rule::ErrorEnum), Verdict::Fail);
}

#[test]
fn rule_two_follows_a_result_alias() {
    let source = "type Result<T> = std::result::Result<T, String>; fn a() -> Result<()> { Ok(()) }";

    assert_eq!(verdict(source, Rule::ErrorEnum), Verdict::Fail);
}

#[test]
fn rule_two_fails_a_library_error_and_trusts_types_from_other_files() {
    assert_eq!(
        verdict(
            "fn a() -> Result<(), std::io::Error> { Ok(()) }",
            Rule::ErrorEnum
        ),
        Verdict::Fail
    );
    assert_eq!(
        verdict(
            "fn a() -> Result<(), DataError> { Ok(()) }",
            Rule::ErrorEnum
        ),
        Verdict::Pass
    );
}

#[test]
fn rule_two_reads_io_result_as_io_error_and_ignores_fmt_result() {
    assert_eq!(
        verdict(
            "fn main() -> std::io::Result<()> { Ok(()) }",
            Rule::ErrorEnum
        ),
        Verdict::Fail
    );

    let display = "struct S; impl std::fmt::Display for S { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) } }";

    assert_eq!(verdict(display, Rule::ErrorEnum), Verdict::NotApplicable);
}

#[test]
fn a_module_declared_by_file_needs_no_doc_at_the_declaration() {
    assert_eq!(
        verdict("pub mod rules;", Rule::PubDocs),
        Verdict::NotApplicable
    );
    assert_eq!(verdict("pub mod inline {}", Rule::PubDocs), Verdict::Fail);
}

#[test]
fn undocumented_pub_items_fail_and_private_ones_do_not_count() {
    assert_eq!(verdict("pub fn a() {}", Rule::PubDocs), Verdict::Fail);
    assert_eq!(
        verdict("/// Adds.\npub fn a() {}", Rule::PubDocs),
        Verdict::Pass
    );
    assert_eq!(verdict("fn a() {}", Rule::PubDocs), Verdict::NotApplicable);
}

#[test]
fn comments_inside_bodies_fail_and_outside_do_not() {
    assert_eq!(
        verdict("// fine\nfn a() {\n    1;\n}", Rule::NoBodyComments),
        Verdict::Pass
    );
    assert_eq!(
        verdict("fn a() {\n    // not fine\n    1;\n}", Rule::NoBodyComments),
        Verdict::Fail
    );
}

#[test]
fn statement_fragments_are_checked_but_their_comments_are_annotations() {
    let report = check("let x = 1; // note\nlet y = x.unwrap();");

    assert!(report.parsed);

    let lines: Vec<(Rule, usize)> = report
        .violations
        .iter()
        .map(|violation| (violation.rule, violation.line))
        .collect();

    assert_eq!(lines, vec![(Rule::NoUnwrap, 2)]);
    assert_eq!(
        report.verdicts.get(&Rule::NoBodyComments),
        Some(&Verdict::NotApplicable)
    );
}

#[test]
fn index_loops_fail_and_counted_repeats_do_not() {
    assert_eq!(
        verdict(
            "fn a(v: &[i32]) { for i in 0..v.len() { let _ = i; } }",
            Rule::NoIndexLoops
        ),
        Verdict::Fail
    );
    assert_eq!(
        verdict(
            "fn a(v: &[i32], n: usize) { for i in 0..n { f(v[i]); } }",
            Rule::NoIndexLoops
        ),
        Verdict::Fail
    );
    assert_eq!(
        verdict("fn a() { for _ in 0..3 { f(); } }", Rule::NoIndexLoops),
        Verdict::Pass
    );
}

#[test]
fn unparsable_code_is_not_judged() {
    let report = check("fn a( {");

    assert!(!report.parsed);
    assert!(report.parse_error.is_some());
    assert!(!report.all_pass());
}
