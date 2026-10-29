//! Reference-checked queries against physical immutable wide parts.
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
fn physical_unsigned_distinct() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-physical_unsigned_distinct-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let schema = [
        Column { name: "id".into(), data_type: "UInt64".into() },
    ];
    let parts: Vec<Vec<Vec<Option<String>>>> = vec![
        vec![vec![Some("0".into())], vec![Some("18446744073709551615".into())], vec![Some("9223372036854775808".into())]],
        vec![vec![Some("18446744073709551615".into())], vec![Some("0".into())], vec![Some("9223372036854775808".into())]],
    ];
    for (index, rows) in parts.iter().enumerate() {
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &schema,
            rows,
        ).unwrap();
    }
    assert_eq!(run(&root, "SELECT count(DISTINCT id) FROM part"), "countDistinct(id)\n3\n");
    fs::remove_dir_all(root).unwrap();
}
