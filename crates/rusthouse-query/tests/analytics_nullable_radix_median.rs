//! Nullable physical integer medians above the legacy in-memory cap.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn nullable_median_selects_exact_non_null_positions_across_100k_rows() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-nullable-radix-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let schema = [Column { name: "value".into(), data_type: "Nullable(UInt64)".into() }];
    for (index, begin, end) in [(1, 0u64, 50_000u64), (2, 50_000, 100_002)] {
        let rows = (begin..end)
            .map(|value| vec![(value < 100_000).then(|| value.to_string())])
            .collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{index}_{index}_0")), &schema, &rows).unwrap();
    }
    let mut out = Vec::new();
    execute(&parse_select("SELECT median(value) FROM part").unwrap(), Some(&root), &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "median(value)\n49999.5\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn all_null_nullable_median_returns_null() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-all-null-radix-{}", std::process::id()
    ));
    let schema = [Column { name: "value".into(), data_type: "Nullable(Int64)".into() }];
    let rows = [vec![None], vec![None]];
    write_wide_part(&root, &schema, &rows).unwrap();
    let mut out = Vec::new();
    execute(&parse_select("SELECT median(value) FROM part").unwrap(), Some(&root), &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "median(value)\nNULL\n");
    fs::remove_dir_all(root).unwrap();
}
