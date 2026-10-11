use super::*;
use crate::testing::TempDir;

fn suggestion(id: &str, rule: &str) -> Suggestion {
    Suggestion {
        id: id.to_string(),
        source: "chat".to_string(),
        rule: rule.to_string(),
        category: SlopCategory::WrapUpRepeat,
        field: Field::Response,
        matched: "in summary".to_string(),
    }
}

#[test]
fn finds_the_first_phrase_of_each_rule_once() {
    let found = findings(
        Field::Response,
        "Great question! Happy to help. In summary, <span>x</span>.",
    );
    let rules: Vec<&str> = found.iter().map(|finding| finding.rule).collect();

    assert_eq!(
        rules,
        ["source_markup", "flattery_filler_opener", "wrap_up_repeat"]
    );
    assert_eq!(found[1].matched, "great question");
}

#[test]
fn counts_em_dashes_only_in_answers() {
    let text = "One — two — three.";

    assert_eq!(findings(Field::Instruction, text), []);
    assert_eq!(
        findings(Field::Response, text),
        [Finding {
            rule: EM_DASH_RULE,
            category: SlopCategory::RhythmTrick,
            matched: "2 em dashes".to_string()
        }]
    );
}

#[test]
fn scans_every_record_and_reports_counts() -> Outcome {
    let folder = TempDir::new()?;
    let records = concat!(
        r#"{"id":"a","source":"chat","origin":"c","instruction":"Great question?","response":"In summary — fine — ok"}"#,
        "\n",
        r#"{"id":"b","source":"book","origin":"x","instruction":"q","response":"plain"}"#,
        "\n"
    );
    let training = folder.file("train.jsonl", records)?;
    let out = folder.path().join("labels/auto.jsonl");
    run(&training, &out)?;

    let written: Vec<Suggestion> = jsonl::read_lines(&out)?;
    let rules: Vec<(&str, Field)> = written
        .iter()
        .map(|found| (found.rule.as_str(), found.field))
        .collect();

    assert_eq!(
        rules,
        [
            ("flattery_filler_opener", Field::Instruction),
            ("wrap_up_repeat", Field::Response),
            (EM_DASH_RULE, Field::Response)
        ]
    );
    assert_eq!(counts(&written).get(EM_DASH_RULE), Some(&1));
    assert_eq!(examples(&written), 1);

    Ok(())
}

#[test]
fn writes_the_auto_flags_format() -> Outcome {
    let line = serde_json::to_string(&suggestion("a", "wrap_up_repeat"))
        .map_err(crate::error::ReviewError::unserializable)?;

    assert_eq!(
        line,
        r#"{"id":"a","source":"chat","rule":"wrap_up_repeat","category":"wrap_up_repeat","field":"response","match":"in summary"}"#
    );

    Ok(())
}

#[test]
fn applying_never_overwrites_a_persons_flag() -> Outcome {
    let folder = TempDir::new()?;
    let mut store = FlagStore::open(&folder.path().join("flags.jsonl"))?;

    store.set(
        "a",
        Flag {
            note: "mine".to_string(),
            spans: Vec::new(),
        },
    );

    let suggestions = [
        suggestion("a", "x"),
        suggestion("b", "wrap_up_repeat"),
        suggestion("b", "other"),
    ];
    let added = apply_to(&suggestions, &mut store)?;

    assert_eq!(added, 1);
    assert_eq!(store.get("a").map(|flag| flag.note.as_str()), Some("mine"));
    assert_eq!(
        store.get("b").map(|flag| flag.note.as_str()),
        Some(
            r#"auto: wrap_up_repeat [wrap_up_repeat] in chat matched "in summary" in response; other [wrap_up_repeat] in chat matched "in summary" in response"#
        )
    );

    Ok(())
}

#[test]
fn apply_reads_and_writes_the_files() -> Outcome {
    let folder = TempDir::new()?;
    let suggestions = folder.path().join("auto.jsonl");
    let flags = folder.path().join("flags.jsonl");

    jsonl::write_lines(&suggestions, &[suggestion("z", "wrap_up_repeat")])?;
    apply(&suggestions, &flags)?;

    assert_eq!(FlagStore::open(&flags)?.len(), 1);

    Ok(())
}
