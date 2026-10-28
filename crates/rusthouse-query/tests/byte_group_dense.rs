use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn dense_unsigned_and_signed_group_counts_preserve_order_filters_and_having() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-byte-group-{}-{}",
        std::process::id(),
        line!()
    ));
    fs::create_dir_all(&root).unwrap();
    let cols = [
        Column {
            name: "key".into(),
            data_type: "UInt8".into(),
        },
        Column {
            name: "signed".into(),
            data_type: "Int8".into(),
        },
    ];
    let rows = vec![
        vec![Some("2".into()), Some("-1".into())],
        vec![Some("1".into()), Some("-2".into())],
        vec![Some("2".into()), Some("-1".into())],
        vec![Some("3".into()), Some("4".into())],
        vec![Some("1".into()), Some("-2".into())],
        vec![Some("2".into()), Some("-1".into())],
    ];
    write_wide_part(&root.join("all_1_1_0"), &cols, &rows).unwrap();
    for (sql, expected) in [
        (
            "SELECT key, count(*) FROM part GROUP BY key",
            "key\tcount\n2\t3\n1\t2\n3\t1\n",
        ),
        (
            "SELECT key, count(*) FROM part WHERE key >= 2 GROUP BY key",
            "key\tcount\n2\t3\n3\t1\n",
        ),
        (
            "SELECT signed, count(*) FROM part GROUP BY signed ORDER BY signed ASC",
            "signed\tcount\n-2\t2\n-1\t3\n4\t1\n",
        ),
        (
            "SELECT key, count(*) FROM part GROUP BY key ORDER BY key DESC LIMIT 2",
            "key\tcount\n3\t1\n2\t3\n",
        ),
    ] {
        let mut result = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut result).unwrap();
        assert_eq!(String::from_utf8(result).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
