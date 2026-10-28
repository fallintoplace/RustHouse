use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn constant_nonnullable_count_filters_skip_rows_without_losing_validation() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-constant-count-{}-{}",
        std::process::id(),
        line!()
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    let rows = (0..3)
        .map(|value| vec![Some(value.to_string())])
        .collect::<Vec<_>>();
    write_wide_part(&root.join("all_1_1_0"), &columns, &rows).unwrap();
    for (sql, expected) in [
        ("SELECT count(*) FROM part WHERE id IS NULL", "count\n0\n"),
        (
            "SELECT count(*) FROM part WHERE id IS NOT NULL",
            "count\n3\n",
        ),
        (
            "SELECT count(*) FROM part WHERE id IS NULL OR id IS NOT NULL",
            "count\n3\n",
        ),
        (
            "SELECT count(*) FROM part WHERE id IS NULL AND id >= 1",
            "count\n0\n",
        ),
        (
            "SELECT count(*) FROM part WHERE id IS NOT NULL AND id >= 1",
            "count\n2\n",
        ),
        (
            "SELECT count(*) FROM part WHERE id IS NULL LIMIT 0",
            "count\n",
        ),
        (
            "SELECT count(*) FROM part WHERE id IS NOT NULL LIMIT 1 OFFSET 1",
            "count\n",
        ),
    ] {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
    let mut result = Vec::new();
    assert!(
        execute(
            &parse_select("SELECT count(*) FROM part WHERE missing IS NULL").unwrap(),
            Some(&root),
            &mut result
        )
        .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}
