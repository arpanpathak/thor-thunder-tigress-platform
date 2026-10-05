//! Finds answers that say they follow the rules when their code does not.
//!
//! The measured baselines claim compliance in most answers ("The design
//! strictly adheres to the following constraints") while breaking the rules
//! they list. A claim is a sentence of prose that matches one of the patterns
//! below. It is false when the rule it names fails; a claim that names no rule
//! is false when any rule fails.

use std::{collections::BTreeMap, sync::LazyLock};

use regex::{Regex, RegexBuilder};
use serde::Serialize;

use crate::rules::{Rule, Verdict};

/// A claim the prose makes that the code contradicts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FalseClaim {
    /// The words of the claim.
    pub text: String,
    /// The byte offset where the claim starts in the answer.
    pub start: usize,
    /// The rule the claim names, or `None` for "follows all the rules".
    pub rule: Option<Rule>,
}

const CLAIMS: [(Option<Rule>, &str); 8] = [
    (
        Some(Rule::NoUnwrap),
        r"\b(?:no (?:use of )?`?\.?(?:unwrap|expect)|never (?:uses?|calls?) `?\.?unwrap|without (?:any )?`?\.?unwrap)",
    ),
    (
        Some(Rule::ErrorEnum),
        r"\bcustom error enum (?:is defined|implement)",
    ),
    (
        Some(Rule::PubDocs),
        r"\b(?:every|all) pub(?:lic)? (?:item|function|type)s? (?:has|have|include|includes|is|are) (?:a )?(?:`///` )?doc",
    ),
    (
        Some(Rule::NoBodyComments),
        r"\bno comments (?:inside|in|within) (?:the )?function bodies",
    ),
    (
        Some(Rule::NoIndexLoops),
        r"\bno index(?:-based)? loops",
    ),
    (
        None,
        r"\b(?:strictly |fully )?(?:adheres|adhering|complies|complying|conforms) (?:to|with) (?:all |the |your |these )*(?:following |given |provided )?(?:constraints|rules|guidelines|requirements)",
    ),
    (
        None,
        r"\bfollows? (?:all |the |your |these )+(?:provided |given )?(?:constraints|rules|guidelines)",
    ),
    (
        None,
        r"\b(?:satisfies|meets) (?:all |every )(?:the )?(?:constraints|rules|requirements)",
    ),
];

static PATTERNS: LazyLock<Vec<(Option<Rule>, Regex)>> = LazyLock::new(|| {
    CLAIMS
        .iter()
        .filter_map(|(rule, pattern)| {
            RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .ok()
                .map(|regex| (*rule, regex))
        })
        .collect()
});

/// The claims in `prose` that the rule verdicts contradict. `prose` must have
/// its code blanked out, as [`crate::slop::prose_only`] does.
pub fn false_claims(prose: &str, verdicts: &BTreeMap<Rule, Verdict>) -> Vec<FalseClaim> {
    let failed = |rule: &Option<Rule>| match rule {
        Some(rule) => verdicts.get(rule) == Some(&Verdict::Fail),
        None => verdicts.values().any(|verdict| *verdict == Verdict::Fail),
    };
    let mut claims: Vec<FalseClaim> = PATTERNS
        .iter()
        .filter(|(rule, _)| failed(rule))
        .flat_map(|(rule, regex)| {
            regex.find_iter(prose).map(move |found| FalseClaim {
                text: found.as_str().to_string(),
                start: found.start(),
                rule: *rule,
            })
        })
        .collect();
    claims.sort_by_key(|claim| claim.start);
    claims
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdicts(failing: &[Rule]) -> BTreeMap<Rule, Verdict> {
        Rule::ALL
            .iter()
            .map(|rule| {
                let verdict = match failing.contains(rule) {
                    true => Verdict::Fail,
                    false => Verdict::Pass,
                };
                (*rule, verdict)
            })
            .collect()
    }

    #[test]
    fn a_claim_about_a_failed_rule_is_false() {
        let prose = "Clean Code: No comments inside function bodies.";
        let found = false_claims(prose, &verdicts(&[Rule::NoBodyComments]));
        assert_eq!(found.len(), 1);
        assert_eq!(found.first().and_then(|claim| claim.rule), Some(Rule::NoBodyComments));
    }

    #[test]
    fn a_claim_about_a_passed_rule_is_not_false() {
        let prose = "No comments inside function bodies.";
        assert!(false_claims(prose, &verdicts(&[Rule::NoUnwrap])).is_empty());
    }

    #[test]
    fn a_general_claim_is_false_when_any_rule_fails() {
        let prose = "The design strictly adheres to the following constraints:";
        assert_eq!(false_claims(prose, &verdicts(&[Rule::PubDocs])).len(), 1);
        assert!(false_claims(prose, &verdicts(&[])).is_empty());
    }
}
