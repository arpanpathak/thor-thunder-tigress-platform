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

/// What a claim says is followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Claim {
    /// One named rule.
    About(Rule),
    /// "Follows all the rules", naming none.
    AllRules,
}

impl Claim {
    /// True when the verdicts contradict the claim.
    fn is_false(self, verdicts: &BTreeMap<Rule, Verdict>) -> bool {
        match self {
            Claim::About(rule) => verdicts.get(&rule) == Some(&Verdict::Fail),
            Claim::AllRules => verdicts.values().any(|verdict| *verdict == Verdict::Fail),
        }
    }

    /// The rule named, as [`FalseClaim::rule`] reports it.
    fn rule(self) -> Option<Rule> {
        match self {
            Claim::About(rule) => Some(rule),
            Claim::AllRules => None,
        }
    }
}

/// The claim each pattern makes.
const CLAIMS: [(Claim, &str); 8] = [
    (
        Claim::About(Rule::NoUnwrap),
        r"\b(?:no (?:use of )?`?\.?(?:unwrap|expect)|never (?:uses?|calls?) `?\.?unwrap|without (?:any )?`?\.?unwrap)",
    ),
    (
        Claim::About(Rule::ErrorEnum),
        r"\bcustom error enum (?:is defined|implement)",
    ),
    (
        Claim::About(Rule::PubDocs),
        r"\b(?:every|all) pub(?:lic)? (?:item|function|type)s? (?:has|have|include|includes|is|are) (?:a )?(?:`///` )?doc",
    ),
    (
        Claim::About(Rule::NoBodyComments),
        r"\bno comments (?:inside|in|within) (?:the )?function bodies",
    ),
    (
        Claim::About(Rule::NoIndexLoops),
        r"\bno index(?:-based)? loops",
    ),
    (
        Claim::AllRules,
        r"\b(?:strictly |fully )?(?:adheres|adhering|complies|complying|conforms) (?:to|with) (?:all |the |your |these )*(?:following |given |provided )?(?:constraints|rules|guidelines|requirements)",
    ),
    (
        Claim::AllRules,
        r"\bfollows? (?:all |the |your |these )+(?:provided |given )?(?:constraints|rules|guidelines)",
    ),
    (
        Claim::AllRules,
        r"\b(?:satisfies|meets) (?:all |every )(?:the )?(?:constraints|rules|requirements)",
    ),
];

/// A claim and the compiled pattern that finds it.
struct ClaimPattern {
    claim: Claim,
    regex: Regex,
}

static PATTERNS: LazyLock<Vec<ClaimPattern>> = LazyLock::new(|| {
    CLAIMS
        .iter()
        .filter_map(|(claim, pattern)| {
            let regex = RegexBuilder::new(pattern).case_insensitive(true).build().ok()?;
            Some(ClaimPattern { claim: *claim, regex })
        })
        .collect()
});

/// The claims in `prose` that the rule verdicts contradict. `prose` must have
/// its code blanked out, as [`crate::slop::prose_only`] does.
#[must_use]
pub fn false_claims(prose: &str, verdicts: &BTreeMap<Rule, Verdict>) -> Vec<FalseClaim> {
    let mut claims: Vec<FalseClaim> = PATTERNS
        .iter()
        .filter(|pattern| pattern.claim.is_false(verdicts))
        .flat_map(|pattern| {
            pattern.regex.find_iter(prose).map(|found| FalseClaim {
                text: found.as_str().to_string(),
                start: found.start(),
                rule: pattern.claim.rule(),
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
                let verdict = if failing.contains(rule) { Verdict::Fail } else { Verdict::Pass };
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
        assert_eq!(false_claims(prose, &verdicts(&[Rule::NoUnwrap])), []);
    }

    #[test]
    fn a_general_claim_is_false_when_any_rule_fails() {
        let prose = "The design strictly adheres to the following constraints:";
        assert_eq!(false_claims(prose, &verdicts(&[Rule::PubDocs])).len(), 1);
        assert_eq!(false_claims(prose, &verdicts(&[])), []);
    }
}
