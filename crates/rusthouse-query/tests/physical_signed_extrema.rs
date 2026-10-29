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
fn physical_signed_extrema() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-physical_signed_extrema-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let schema = [
        Column { name: "id".into(), data_type: "Int64".into() },
    ];
    let parts: Vec<Vec<Vec<Option<String>>>> = vec![
        vec![vec![Some("-10".into())], vec![Some("4".into())], vec![Some("-3".into())]],
        vec![vec![Some("0".into())], vec![Some("99".into())], vec![Some("-100".into())]],
    ];
    for (index, rows) in parts.iter().enumerate() {
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &schema,
            rows,
        ).unwrap();
    }
    assert_eq!(run(&root, "SELECT min(id) FROM part"), "min(id)\n-100\n");
    assert_eq!(run(&root, "SELECT max(id) FROM part"), "max(id)\n99\n");
    assert_eq!(run(&root, "SELECT count(*) FROM part WHERE id < 0"), "count\n3\n");
    fs::remove_dir_all(root).unwrap();
}
