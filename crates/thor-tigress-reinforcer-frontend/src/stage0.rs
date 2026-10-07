//! What Stage 0 (`spark`) finds in one answer, in the shape the page draws:
//! slop phrases to suggest, and code lines that break one of the five rules.

use serde::Serialize;
use thor_spark_safety_eval::answer;

use crate::{category::SlopCategory, titles};

/// The sources written by the reviewer. They are the reference, and they show
/// bad code on purpose to contrast it with good code, so the page suggests
/// nothing in them.
const OWN_SOURCES: [&str; 2] = ["readability", "clever_vs_readable"];

/// Everything Stage 0 suggests for one answer.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Suggestions {
    slop: Vec<SlopHit>,
    violations: Vec<Violation>,
}

/// A slop phrase Stage 0 found.
#[derive(Debug, PartialEq, Eq, Serialize)]
struct SlopHit {
    text: String,
    category: SlopCategory,
}

/// A code line that breaks one of the five rules. The line is sent as its
/// text, so the page can find it in whatever block it renders without both
/// sides agreeing on how blocks are counted.
#[derive(Debug, PartialEq, Eq, Serialize)]
struct Violation {
    rule: &'static str,
    detail: String,
    line: String,
}

/// Stage 0's suggestions for `response`, from `source`; none for the
/// reviewer's own sources.
#[must_use]
pub fn suggestions(source: &str, response: &str) -> Suggestions {
    if OWN_SOURCES.contains(&source) {
        return Suggestions::default();
    }
    let score = answer::score(response);
    let lines: Vec<&str> = response.lines().collect();
    let slop = score
        .slop
        .hits
        .into_iter()
        .map(|hit| SlopHit {
            text: hit.text,
            category: hit.category.into(),
        })
        .collect();
    let mut violations = Vec::new();
    for block in score.blocks {
        for violation in block.report.violations {
            let line = (block.line + violation.line)
                .checked_sub(2)
                .and_then(|index| lines.get(index));
            violations.push(Violation {
                rule: titles::rule_title(violation.rule),
                detail: violation.detail,
                line: line.map_or("", |text| text.trim()).to_string(),
            });
        }
    }
    Suggestions { slop, violations }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_slop_and_rule_breaks_with_the_line_text() {
        let answer = "Great question! Here it is.\n```rust\nfn main() {\n    let x = run().unwrap();\n}\n```\n";
        let found = suggestions("chat", answer);
        assert!(
            found
                .slop
                .iter()
                .any(|hit| hit.category == SlopCategory::FlatteryFillerOpener)
        );
        let unwrap = found
            .violations
            .iter()
            .find(|violation| violation.rule == "Calls unwrap() or expect()");
        assert_eq!(
            unwrap.map(|violation| violation.line.as_str()),
            Some("let x = run().unwrap();")
        );
    }

    #[test]
    fn suggests_nothing_in_the_reviewers_own_sources() {
        assert_eq!(
            suggestions("readability", "Great question! x.unwrap()"),
            Suggestions::default()
        );
    }
}
