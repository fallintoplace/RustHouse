//! Persisted physical-part query correctness regression.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::Path;

fn run(root: &Path, sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(root), &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn physical_signed_extrema() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-physical_signed_extrema-{}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();

    let schema = [
        Column {
            name: "id".into(),
            data_type: "Int64".into(),
        },
    ];
    let parts: Vec<Vec<Vec<Option<String>>>> = vec![
        vec![
            vec![Some("-10".into())],
            vec![Some("4".into())],
            vec![Some("-3".into())],
        ],
        vec![
            vec![Some("0".into())],
            vec![Some("99".into())],
            vec![Some("-100".into())],
        ],
    ];
    for (index, rows) in parts.iter().enumerate() {
        let part = root.join(format!("all_{index}_{index}_0"));
        write_wide_part(&part, &schema, rows).unwrap();
    }

    let sql = "SELECT min(id) FROM part";
    let expected = "min(id)\n-100\n";
    assert_eq!(run(&root, sql), expected, "{sql}");

    let sql = "SELECT max(id) FROM part";
    let expected = "max(id)\n99\n";
    assert_eq!(run(&root, sql), expected, "{sql}");

    let sql = "SELECT count(*) FROM part WHERE id < 0";
    let expected = "count\n3\n";
    assert_eq!(run(&root, sql), expected, "{sql}");
    fs::remove_dir_all(root).unwrap();
}
