//! Exact median on physical snapshots above the in-memory 100K row limit.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn exact_radix_median_spans_large_snapshot_parts() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-radix-median-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let columns = [Column { name: "id".into(), data_type: "UInt64".into() }];
    for (part_no, begin, end) in [(1, 0u64, 50_000u64), (2, 50_000, 100_001)] {
        let rows = (begin..end).map(|v| vec![Some(v.to_string())]).collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{part_no}_{part_no}_0")), &columns, &rows).unwrap();
    }
    let mut output = Vec::new();
    execute(&parse_select("SELECT median(id) FROM part").unwrap(), Some(&root), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "median(id)\n50000\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn signed_median_respects_negative_key_order_across_parts() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-radix-signed-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let columns = [Column { name: "amount".into(), data_type: "Int64".into() }];
    for (part_no, begin, end) in [(1, -50_001i64, 0i64), (2, 0, 50_001)] {
        let rows = (begin..end).map(|v| vec![Some(v.to_string())]).collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{part_no}_{part_no}_0")), &columns, &rows).unwrap();
    }
    let mut output = Vec::new();
    execute(&parse_select("SELECT median(amount) FROM part").unwrap(), Some(&root), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "median(amount)\n-0.5\n");
    fs::remove_dir_all(root).unwrap();
}
