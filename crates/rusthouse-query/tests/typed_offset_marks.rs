use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn typed_offset_skips_full_marks_without_discarding_partial_mark() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-typed-offset-{}-{}",
        std::process::id(),
        line!(),
    ));
    fs::create_dir_all(&root).unwrap();
    let part = root.join("all_1_1_0");
    let columns = vec![
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "label".into(),
            data_type: "String".into(),
        },
    ];
    let rows = (0..2050)
        .map(|row| vec![Some(row.to_string()), Some(format!("v-{row}"))])
        .collect::<Vec<_>>();
    write_wide_part(&part, &columns, &rows).unwrap();
    let cases = [
        (
            "SELECT label FROM part LIMIT 2 OFFSET 2047",
            "label\nv-2047\nv-2048\n",
        ),
        (
            "SELECT label FROM part LIMIT 3 OFFSET 2048",
            "label\nv-2048\nv-2049\n",
        ),
        ("SELECT label FROM part LIMIT 2 OFFSET 2050", "label\n"),
        ("SELECT label FROM part LIMIT 1 OFFSET 2051", "label\n"),
        (
            "SELECT label FROM part WHERE id >= 2048 LIMIT 1 OFFSET 1",
            "label\nv-2049\n",
        ),
    ];
    for (sql, expected) in cases {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
