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
fn physical_granule_boundary() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-physical_granule_boundary-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let schema = [
        Column { name: "id".into(), data_type: "UInt64".into() },
    ];
    let rows = (0..=1024u64)
        .map(|value| vec![Some(value.to_string())])
        .collect::<Vec<_>>();
    write_wide_part(&root.join("all_0_0_0"), &schema, &rows).unwrap();
    assert_eq!(run(&root, "SELECT count(*) FROM part WHERE id % 7 = 0"), "count\n147\n");
    assert_eq!(run(&root, "SELECT sum(id) FROM part"), "sum(id)\n524800\n");
    fs::remove_dir_all(root).unwrap();
}
