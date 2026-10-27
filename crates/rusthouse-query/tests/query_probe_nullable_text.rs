use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn nullable_text_predicates_keep_case_escapes_and_null_semantics() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-query-text-probe-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = [
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "tag".into(),
            data_type: "Nullable(String)".into(),
        },
        Column {
            name: "score".into(),
            data_type: "Nullable(Int64)".into(),
        },
    ];
    let batches: [[(u64, Option<&str>, Option<i64>); 4]; 2] = [
        [
            (1, Some("Alpha"), Some(12)),
            (2, Some("beta"), None),
            (3, None, Some(-5)),
            (4, Some("A_1"), Some(0)),
        ],
        [
            (5, Some("alphabet"), Some(30)),
            (6, Some("ALPS"), None),
            (7, Some("beta"), Some(7)),
            (8, Some("A_2"), Some(5)),
        ],
    ];
    for (index, batch) in batches.iter().enumerate() {
        let rows = batch
            .iter()
            .map(|(id, tag, score)| {
                vec![
                    Some(id.to_string()),
                    tag.map(str::to_owned),
                    score.map(|value| value.to_string()),
                ]
            })
            .collect::<Vec<_>>();
        let part = index + 1;
        write_wide_part(
            &root.join(format!("all_{part}_{part}_0")),
            &columns,
            &rows,
        )
        .unwrap();
    }
    for (sql, expected) in [
        (
            "SELECT id FROM part WHERE tag ILIKE 'al%' ORDER BY id",
            "id\n1\n5\n6\n",
        ),
        (
            r"SELECT id FROM part WHERE tag LIKE 'A\_%' ORDER BY id",
            "id\n4\n8\n",
        ),
        (
            "SELECT id FROM part WHERE score IS NULL OR score < 0 ORDER BY id DESC",
            "id\n6\n3\n2\n",
        ),
        (
            "SELECT id FROM part WHERE tag NOT LIKE 'A%' ORDER BY id",
            "id\n2\n5\n7\n",
        ),
    ] {
        let parsed = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
        let mut output = Vec::new();
        execute(&parsed, Some(&root), &mut output)
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
