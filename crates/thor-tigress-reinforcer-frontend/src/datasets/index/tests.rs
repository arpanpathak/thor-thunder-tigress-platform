use super::*;

#[test]
fn more_sources_than_ids_is_refused() -> Outcome {
    let folder = TempDir::new()?;
    let lines: String = (0..=usize::from(u16::MAX) + 1)
        .map(|number| {
            format!("{{\"id\":\"{number}\",\"source\":\"s{number}\",\"origin\":\"o\"}}\n")
        })
        .collect();
    let path = folder.file("train.jsonl", &lines)?;

    assert!(
        matches!(Index::open(&path), Err(ReviewError::BadRequest(message)) if message.contains("too many"))
    );

    Ok(())
}

#[test]
fn a_record_changed_into_invalid_text_after_indexing_is_refused() -> Outcome {
    let folder = TempDir::new()?;
    let line = "{\"id\":\"a\",\"source\":\"chat\",\"origin\":\"c\"}\n";
    let path = folder.file("train.jsonl", line)?;
    let index = Index::open(&path)?;

    std::fs::write(&path, [0xff_u8; 64]).map_err(ReviewError::io(&path))?;

    assert!(
        matches!(index.record(0), Err(ReviewError::BadRequest(message)) if message.contains("not UTF-8"))
    );

    Ok(())
}
use crate::testing::TempDir;

#[test]
fn equally_large_collections_come_in_folder_order() -> Outcome {
    let folder = TempDir::new()?;
    let lines = concat!(
        r#"{"id":"r","source":"corpus","origin":"rbe/src/a.md"}"#,
        "\n",
        r#"{"id":"t","source":"corpus","origin":"trpl/src/a.md"}"#,
        "\n"
    );
    let index = Index::open(&folder.file("train.jsonl", lines)?)?;
    let folders: Vec<String> = index
        .collections()
        .into_iter()
        .map(|collection| collection.folder)
        .collect();

    assert_eq!(folders, ["rbe", "trpl"]);

    Ok(())
}

const LINES: &str = concat!(
    r#"{"id":"a1","instruction":"What is a page?","response":"A block.","source":"book","origin":"ch01.md"}"#,
    "\n",
    r#"{"id":"b2","instruction":"What is a cache?","response":"Fast memory.","source":"chat","origin":"c1"}"#,
    "\n\n",
    r#"{"id":"c3","instruction":"What is a page table?","response":"Maps pages.","source":"book","origin":"ch02.md"}"#,
    "\n"
);

const BOOKS: &str = concat!(
    r#"{"id":"t1","instruction":"a","response":"b","source":"corpus","origin":"trpl/src/ch01.md"}"#,
    "\n",
    r#"{"id":"t2","instruction":"a","response":"b","source":"corpus","origin":"trpl/src/ch02.md"}"#,
    "\n",
    r#"{"id":"r1","instruction":"a","response":"b","source":"corpus","origin":"rbe/src/hello.md"}"#,
    "\n",
    r#"{"id":"c1","instruction":"a","response":"b","source":"chat","origin":"conversations.json#x/1"}"#,
    "\n"
);

fn index_of(folder: &TempDir, lines: &str) -> Outcome<Index> {
    Index::open(&folder.file("train.jsonl", lines)?)
}

fn filter(apply: impl FnOnce(&mut Filter)) -> Filter {
    let mut filter = Filter::default();
    apply(&mut filter);

    filter
}

#[test]
fn indexes_every_record_with_its_source() -> Outcome {
    let folder = TempDir::new()?;
    let index = index_of(&folder, LINES)?;

    assert_eq!(index.len(), 3);
    assert!(index.contains("b2") && !index.contains("zz"));
    assert_eq!(
        index.counts(),
        [
            SourceCount {
                name: "book".to_string(),
                count: 2
            },
            SourceCount {
                name: "chat".to_string(),
                count: 1
            }
        ]
    );
    assert_eq!(
        index.entry(1).map(|entry| index.origin_of(entry)),
        Some("c1")
    );

    Ok(())
}

#[test]
fn reads_a_record_back_whole_and_parsed() -> Outcome {
    #[derive(Deserialize)]
    struct Answer {
        response: String,
    }

    let folder = TempDir::new()?;
    let index = index_of(&folder, LINES)?;

    assert!(index.record(1)?.starts_with(r#"{"id":"b2""#));
    assert_eq!(index.parsed::<Answer>(2)?.response, "Maps pages.");
    assert!(matches!(index.record(9), Err(ReviewError::NotFound(_))));

    Ok(())
}

#[test]
fn filters_by_source_and_flag_without_reading_the_file() -> Outcome {
    let folder = TempDir::new()?;
    let index = index_of(&folder, LINES)?;
    let flagged: Ids = ["b2".to_string()].into_iter().collect();

    assert_eq!(
        index.matching(&filter(|f| f.source = Some("book".to_string())), &flagged)?,
        [0, 2]
    );
    assert_eq!(
        index.matching(&filter(|f| f.flagged = Some(true)), &flagged)?,
        [1]
    );
    assert_eq!(
        index.matching(&filter(|f| f.flagged = Some(false)), &flagged)?,
        [0, 2]
    );

    Ok(())
}

#[test]
fn searches_the_whole_record_ignoring_case() -> Outcome {
    let folder = TempDir::new()?;
    let index = index_of(&folder, LINES)?;
    let spec = filter(|f| f.query = Some("PAGE TABLE".to_string()));
    let found = index.matching(&spec, &Ids::new())?;

    assert_eq!(found, [2]);

    Ok(())
}

#[test]
fn finds_a_query_that_contains_a_quote() -> Outcome {
    let folder = TempDir::new()?;
    let line = concat!(
        r#"{"id":"q1","instruction":"x","response":"say \"hi\" now","source":"chat","origin":"c"}"#,
        "\n"
    );
    let index = index_of(&folder, line)?;
    let spec = filter(|f| f.query = Some("say \"hi\"".to_string()));
    let found = index.matching(&spec, &Ids::new())?;

    assert_eq!(found, [0]);

    Ok(())
}

#[test]
fn knows_which_book_a_record_came_from() -> Outcome {
    let folder = TempDir::new()?;
    let index = index_of(&folder, BOOKS)?;
    let spec = filter(|f| f.collection = Some("trpl".to_string()));
    let found = index.matching(&spec, &Ids::new())?;

    assert_eq!(found, [0, 1]);
    assert_eq!(
        index.collections(),
        [
            CollectionCount {
                folder: "trpl".to_string(),
                source: "corpus".to_string(),
                count: 2
            },
            CollectionCount {
                folder: "rbe".to_string(),
                source: "corpus".to_string(),
                count: 1
            },
        ]
    );
    assert_eq!(
        index.entry(3).and_then(|entry| index.collection_of(entry)),
        None
    );

    Ok(())
}

#[test]
fn a_line_without_its_header_fields_is_named() -> Outcome {
    let folder = TempDir::new()?;
    let outcome = index_of(&folder, "{\"id\":\"a\"}\n");

    assert!(outcome.is_err_and(|error| error.to_string().contains("train.jsonl:1:")));

    Ok(())
}

#[test]
fn the_interner_numbers_each_name_once() {
    let mut interner: Interner<u16> = Interner::new();

    assert_eq!(
        [
            interner.id_of("book"),
            interner.id_of("chat"),
            interner.id_of("book")
        ],
        [Some(0), Some(1), Some(0)]
    );
    assert_eq!(interner.name(1), "chat");
    assert_eq!(interner.name(7), "");
}

#[test]
fn the_interner_refuses_more_names_than_its_ids_can_count() {
    let mut interner: Interner<u8> = Interner::new();
    let ids: Vec<Option<u8>> = (0..=256).map(|n| interner.id_of(&n.to_string())).collect();

    assert_eq!(ids[255], Some(255));
    assert_eq!(ids[256], None);
}

#[test]
fn escapes_a_query_like_json_does() {
    assert_eq!(json_escaped(r#"say "hi""#), r#"say \"hi\""#);
    assert_eq!(json_escaped("a\\b"), "a\\\\b");
}
