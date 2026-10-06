//! Scores one whole answer: its prose for slop and its Rust blocks for the
//! five rules.

use std::{collections::BTreeMap, sync::LazyLock};

use regex::Regex;
use serde::Serialize;

use crate::{
    claims::{self, FalseClaim},
    rules::{self, CodeReport, Rule, Verdict},
    slop::{self, SlopReport},
};

/// A Rust code block found in an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    /// The 1-based line of the answer the code starts on.
    pub line: usize,
    /// The code, without its fences.
    pub code: String,
}

/// One Rust block of an answer and what the rules said about it.
#[derive(Debug, Clone, Serialize)]
pub struct Block {
    /// The 1-based line of the answer the block's code starts on.
    pub line: usize,
    /// The rule report for the block. Violation lines count from the block.
    pub report: CodeReport,
}

/// Everything Stage 0 says about one answer.
#[derive(Debug, Clone, Serialize)]
pub struct AnswerScore {
    /// Slop in the prose.
    pub slop: SlopReport,
    /// The Rust blocks, in order.
    pub blocks: Vec<Block>,
    /// One verdict per rule over all blocks: `fail` if any block fails it,
    /// `pass` if at least one block passes and none fails.
    pub rules: BTreeMap<Rule, Verdict>,
    /// True when the answer has Rust code, every block parses and no rule
    /// fails; `None` when there is no Rust code to judge.
    pub all_rules: Option<bool>,
    /// Claims in the prose that the rule verdicts contradict.
    pub false_claims: Vec<FalseClaim>,
}

static FENCED: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?ms)^[ \t]*(?:```|~~~)[ \t]*([\w+#.-]*)[^\n]*\n(.*?)(?:^[ \t]*(?:```|~~~)[ \t]*$|\z)").ok()
});

static RUST_HINT: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\b(?:fn|let|impl|use|struct|enum|pub|mod|match)\b").ok());

/// The Rust blocks of a markdown answer with the line each starts on. A block
/// counts when its fence says `rust` or `rs`, or it has no language and reads
/// like Rust (it contains `fn`, `let`, `impl` and the like).
#[must_use]
pub fn rust_blocks(text: &str) -> Vec<CodeBlock> {
    let (Some(fenced), Some(hint)) = (FENCED.as_ref(), RUST_HINT.as_ref()) else {
        return Vec::new();
    };
    fenced
        .captures_iter(text)
        .filter_map(|captures| {
            let language = captures.get(1).map_or("", |found| found.as_str());
            let code = captures.get(2)?;
            let is_rust = matches!(language, "rust" | "rs")
                || (language.is_empty() && hint.is_match(code.as_str()));
            let line = text[..code.start()].matches('\n').count() + 1;
            is_rust.then(|| CodeBlock { line, code: code.as_str().to_string() })
        })
        .collect()
}

/// Scores a markdown answer: slop in the prose, rules in the Rust blocks.
#[must_use]
pub fn score(text: &str) -> AnswerScore {
    let blocks = rust_blocks(text)
        .into_iter()
        .map(|block| Block { line: block.line, report: rules::check(&block.code) })
        .collect();
    combine(text, blocks)
}

/// Scores an answer whose code is already separate from its prose, such as
/// an eval run that stored the extracted code in its own field.
#[must_use]
pub fn score_parts(prose: &str, code: &str) -> AnswerScore {
    let blocks = if code.trim().is_empty() {
        Vec::new()
    } else {
        vec![Block { line: 1, report: rules::check(code) }]
    };
    combine(prose, blocks)
}

fn combine(prose: &str, blocks: Vec<Block>) -> AnswerScore {
    let rules = Rule::ALL
        .iter()
        .map(|rule| {
            let verdicts: Vec<Verdict> = blocks
                .iter()
                .filter_map(|block| block.report.verdicts.get(rule).copied())
                .collect();
            let verdict = match (
                verdicts.contains(&Verdict::Fail),
                verdicts.contains(&Verdict::Pass),
            ) {
                (true, _) => Verdict::Fail,
                (false, true) => Verdict::Pass,
                (false, false) => Verdict::NotApplicable,
            };
            (*rule, verdict)
        })
        .collect();
    let all_rules = (!blocks.is_empty()).then(|| blocks.iter().all(|block| block.report.all_pass()));
    let false_claims = claims::false_claims(&slop::prose_only(prose), &rules);
    AnswerScore {
        slop: slop::check(prose),
        blocks,
        rules,
        all_rules,
        false_claims,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_labelled_and_unlabelled_rust_blocks_but_not_shell() {
        let text = "Intro\n```rust\nfn a() {}\n```\n```\nlet x = 1;\n```\n```sh\ncargo run\n```\n";
        let lines: Vec<usize> = rust_blocks(text).into_iter().map(|block| block.line).collect();
        assert_eq!(lines, vec![3, 6]);
    }

    #[test]
    fn an_answer_without_code_has_no_rule_verdict() {
        let scored = score("Use an iterator here.");
        assert_eq!(scored.all_rules, None);
        assert!(scored.rules.values().all(|verdict| *verdict == Verdict::NotApplicable));
    }

    #[test]
    fn one_failing_block_fails_the_answer() {
        let text = "```rust\n/// Ok.\npub fn a() {}\n```\n```rust\nfn b() { c().unwrap(); }\n```";
        let scored = score(text);
        assert_eq!(scored.all_rules, Some(false));
        assert_eq!(scored.rules.get(&Rule::NoUnwrap), Some(&Verdict::Fail));
        assert_eq!(scored.rules.get(&Rule::PubDocs), Some(&Verdict::Pass));
    }

    #[test]
    fn a_compliance_claim_over_broken_code_is_false() {
        let text = "No comments inside function bodies.\n```rust\nfn a() {\n    // note\n}\n```";
        assert_eq!(score(text).false_claims.len(), 1);
    }

    #[test]
    fn code_given_apart_from_the_prose_is_one_block() {
        let scored = score_parts("Plain prose.", "fn a() { b().unwrap(); }");
        assert_eq!(scored.blocks.len(), 1);
        assert_eq!(scored.rules.get(&Rule::NoUnwrap), Some(&Verdict::Fail));
        assert_eq!(score_parts("Prose.", "  ").all_rules, None);
    }

    #[test]
    fn an_unclosed_fence_still_yields_its_code() {
        assert_eq!(rust_blocks("```rust\nfn a() {}\n").len(), 1);
    }
}
