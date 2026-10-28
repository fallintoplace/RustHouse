//! Filtered radix selection on physical fact data larger than the old cap.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

fn run(path: &std::path::Path, sql: &str) -> String {
    let mut out = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(path), &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn large_filtered_median_avoids_collecting_120k_rows() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-filtered-radix-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let schema = [Column { name: "id".into(), data_type: "UInt64".into() }];
    for (part_no, begin, end) in [(1, 0u64, 60_000u64), (2, 60_000, 120_001)] {
        let rows = (begin..end).map(|v| vec![Some(v.to_string())]).collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{part_no}_{part_no}_0")), &schema, &rows).unwrap();
    }
    assert_eq!(
        run(&root, "SELECT median(id) FROM part WHERE id >= 100"),
        "median(id)\n60050\n"
    );
    assert_eq!(
        run(&root, "SELECT median(id) FROM part WHERE id > 120000"),
        "median(id)\nNULL\n"
    );
    fs::remove_dir_all(root).unwrap();
}
