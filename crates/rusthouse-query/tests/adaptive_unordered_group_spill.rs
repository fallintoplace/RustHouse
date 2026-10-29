use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn unordered_high_cardinality_u64_group_by_spills_only_after_hash_overflows() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-unordered-u64-spill-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let schema = [Column {name:"user_id".into(), data_type:"UInt64".into()}];
    let rows = (0..210_000u64).map(|id| vec![Some(id.to_string())])
        .collect::<Vec<_>>();
    write_wide_part(&root, &schema, &rows).unwrap();
    let query = parse_select("SELECT user_id, count(*) FROM part GROUP BY user_id").unwrap();
    let mut result = Vec::new();
    execute(&query, Some(&root), &mut result).unwrap();
    let value = String::from_utf8(result).unwrap();
    let lines = value.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 210_001);
    assert_eq!(lines[0], "user_id\tcount");
    assert_eq!(lines[1], "0\t1");
    assert_eq!(lines.last().copied(), Some("209999\t1"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ordinary_small_unordered_grouping_preserves_first_seen_order() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-small-u64-group-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let schema = [Column {name:"user_id".into(), data_type:"UInt64".into()}];
    write_wide_part(&root, &schema, &[
        vec![Some("7".into())], vec![Some("2".into())],
        vec![Some("7".into())], vec![Some("1".into())],
    ]).unwrap();
    let query = parse_select("SELECT user_id, count(*) FROM part GROUP BY user_id").unwrap();
    let mut result = Vec::new();
    execute(&query, Some(&root), &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(),
        "user_id\tcount\n7\t2\n2\t1\n1\t1\n");
    fs::remove_dir_all(root).unwrap();
}
