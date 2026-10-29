//! Exact results from multiple immutable physical parts.
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
fn physical_topk_offset() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-physical_topk_offset-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let schema = [
        Column { name: "id".into(), data_type: "UInt64".into() },
    ];
    let parts: Vec<Vec<Vec<Option<String>>>> = vec![
        vec![vec![Some("3".into())], vec![Some("1".into())], vec![Some("6".into())]],
        vec![vec![Some("2".into())], vec![Some("5".into())], vec![Some("4".into())]],
    ];
    for (index, rows) in parts.iter().enumerate() {
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &schema,
            rows,
        ).unwrap();
    }
    assert_eq!(run(&root, "SELECT id FROM part ORDER BY id DESC LIMIT 3 OFFSET 1"), "id\n5\n4\n3\n");
    assert_eq!(run(&root, "SELECT id FROM part ORDER BY id ASC LIMIT 2 OFFSET 3"), "id\n4\n5\n");
    fs::remove_dir_all(root).unwrap();
}
