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
fn physical_nullable_predicates() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-physical_nullable_predicates-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let schema = [
        Column { name: "amount".into(), data_type: "Nullable(UInt64)".into() },
    ];
    let parts: Vec<Vec<Vec<Option<String>>>> = vec![
        vec![vec![None], vec![Some("10".into())], vec![None]],
        vec![vec![Some("5".into())], vec![None], vec![Some("12".into())]],
    ];
    for (index, rows) in parts.iter().enumerate() {
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &schema,
            rows,
        ).unwrap();
    }
    assert_eq!(run(&root, "SELECT count(*) FROM part WHERE amount IS NULL"), "count\n3\n");
    assert_eq!(run(&root, "SELECT count(*) FROM part WHERE amount IS NOT NULL"), "count\n3\n");
    assert_eq!(run(&root, "SELECT count(amount) FROM part"), "count(amount)\n3\n");
    fs::remove_dir_all(root).unwrap();
}
