use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn nonnullable_typed_null_filters_are_exact_without_decoding_values() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-typed-constant-null-{}-{}",
        std::process::id(),
        line!()
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = [
        Column {
            name: "label".into(),
            data_type: "String".into(),
        },
        Column {
            name: "nullable".into(),
            data_type: "Nullable(String)".into(),
        },
    ];
    let rows = vec![
        vec![Some("x".into()), None],
        vec![Some("y".into()), Some("a".into())],
        vec![Some("z".into()), None],
    ];
    write_wide_part(&root.join("all_1_1_0"), &columns, &rows).unwrap();
    for (sql, expected) in [
        (
            "SELECT count(*) FROM part WHERE label IS NULL",
            "count\n0\n",
        ),
        (
            "SELECT count(*) FROM part WHERE label IS NOT NULL",
            "count\n3\n",
        ),
        (
            "SELECT count(*) FROM part WHERE label IS NULL OR label IS NOT NULL",
            "count\n3\n",
        ),
        (
            "SELECT count(*) FROM part WHERE label IS NULL AND nullable IS NOT NULL",
            "count\n0\n",
        ),
        (
            "SELECT count(*) FROM part WHERE nullable IS NULL",
            "count\n2\n",
        ),
        (
            "SELECT count(*) FROM part WHERE label IS NOT NULL AND nullable IS NOT NULL",
            "count\n1\n",
        ),
        (
            "SELECT count(*) FROM part WHERE label IS NOT NULL LIMIT 1 OFFSET 1",
            "count\n",
        ),
    ] {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
