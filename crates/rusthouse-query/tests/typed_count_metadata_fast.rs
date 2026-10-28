use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn typed_count_uses_non_nullable_row_count_but_keeps_nullable_semantics() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-typed-metadata-count-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = [
        Column {
            name: "label".into(),
            data_type: "String".into(),
        },
        Column {
            name: "optional".into(),
            data_type: "Nullable(String)".into(),
        },
    ];
    let rows = vec![
        vec![Some("a".into()), Some("a".into())],
        vec![Some("b".into()), None],
        vec![Some("c".into()), Some("".into())],
    ];
    write_wide_part(&root.join("all_0_0_0"), &columns, &rows).unwrap();
    for (sql, expected) in [
        ("SELECT count(label) FROM part", "count(label)\n3\n"),
        ("SELECT count(optional) FROM part", "count(optional)\n2\n"),
        ("SELECT count(label) FROM part LIMIT 0", "count(label)\n"),
        (
            "SELECT count(label) FROM part LIMIT 1 OFFSET 1",
            "count(label)\n",
        ),
        (
            "SELECT count(label) FROM part WHERE label = 'a'",
            "count(label)\n1\n",
        ),
    ] {
        let mut result = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut result).unwrap();
        assert_eq!(String::from_utf8(result).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
