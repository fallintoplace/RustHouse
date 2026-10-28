use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn length_offset_uses_mark_seek_and_preserves_filtered_fallback() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-length-offset-{}-{}",
        std::process::id(),
        line!()
    ));
    fs::create_dir_all(&root).unwrap();
    let cols = [
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "value".into(),
            data_type: "String".into(),
        },
    ];
    let rows = (0..2050usize)
        .map(|row| vec![Some(row.to_string()), Some(format!("row-{row}"))])
        .collect::<Vec<_>>();
    write_wide_part(&root.join("all_1_1_0"), &cols, &rows).unwrap();
    for (sql, expected) in [
        (
            "SELECT length(value) FROM part LIMIT 2 OFFSET 2048",
            "length(value)\n8\n8\n",
        ),
        (
            "SELECT length(value) FROM part LIMIT 1 OFFSET 2050",
            "length(value)\n",
        ),
        (
            "SELECT length(value) FROM part WHERE id >= 2049 LIMIT 1",
            "length(value)\n8\n",
        ),
        (
            "SELECT length(value) FROM part WHERE id >= 2047 LIMIT 1 OFFSET 2",
            "length(value)\n8\n",
        ),
    ] {
        let mut result = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut result).unwrap();
        assert_eq!(String::from_utf8(result).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
