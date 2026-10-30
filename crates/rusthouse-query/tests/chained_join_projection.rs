use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_join_skips_unused_huge_dimension_payload_columns() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-chained-pruning-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let a = root.join("a");
    let b = root.join("b");
    let c = root.join("c");
    let schema = [
        Column { name: "id".into(), data_type: "UInt64".into() },
        Column { name: "payload".into(), data_type: "String".into() },
    ];
    write_wide_part(&a, &schema, &[vec![Some("1".into()), Some("fact".into())]]).unwrap();
    write_wide_part(&b, &schema, &[vec![Some("1".into()), Some("unused-b".repeat(1000))]]).unwrap();
    write_wide_part(&c, &schema, &[vec![Some("1".into()), Some("unused-c".repeat(1000))]]).unwrap();
    let query = parse_join(
        "SELECT a.id FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id"
    ).unwrap().unwrap();
    let mut out = Vec::new();
    execute_multi_join(&query, &[a, b, c], &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "a.id\n1\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn chained_star_retains_all_columns_and_order_by_reads_unprojected_sort_keys() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-chained-order-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let schema = [
        Column { name: "id".into(), data_type: "UInt64".into() },
        Column { name: "payload".into(), data_type: "String".into() },
    ];
    let paths = ["a", "b", "c"].iter().map(|name| {
        let path = root.join(name);
        write_wide_part(&path, &schema, &[vec![Some("1".into()), Some(format!("from-{name}"))]]).unwrap();
        path
    }).collect::<Vec<_>>();
    let query = parse_join(
        "SELECT a.id FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id ORDER BY c.payload DESC"
    ).unwrap().unwrap();
    let mut out = Vec::new();
    execute_multi_join(&query, &paths, &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "a.id\n1\n");

    let query = parse_join(
        "SELECT * FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id"
    ).unwrap().unwrap();
    let mut out = Vec::new();
    execute_multi_join(&query, &paths, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("from-a\t1\tfrom-b\t1\tfrom-c\n"), "{text}");
    fs::remove_dir_all(root).unwrap();
}
