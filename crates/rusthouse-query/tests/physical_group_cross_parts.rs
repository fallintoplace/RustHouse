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
fn physical_group_cross_parts() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-physical_group_cross_parts-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let schema = [
        Column { name: "region".into(), data_type: "String".into() },
    ];
    let parts: Vec<Vec<Vec<Option<String>>>> = vec![
        vec![vec![Some("EU".into())], vec![Some("NA".into())], vec![Some("EU".into())]],
        vec![vec![Some("APAC".into())], vec![Some("NA".into())], vec![Some("EU".into())]],
    ];
    for (index, rows) in parts.iter().enumerate() {
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &schema,
            rows,
        ).unwrap();
    }
    assert_eq!(run(&root, "SELECT region, count(*) FROM part GROUP BY region ORDER BY region"), "region\tcount\nAPAC\t1\nEU\t3\nNA\t2\n");
    fs::remove_dir_all(root).unwrap();
}
