use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_inner_count_scans_last_bucket_without_retaining_join_product() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-chain-count-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let schema = [Column { name: "id".into(), data_type: "UInt64".into() }];
    let mut paths = Vec::new();
    for (name, n) in [("a", 2), ("b", 2), ("c", 5)] {
        let path = root.join(name);
        let rows = (0..n).map(|_| vec![Some("7".into())]).collect::<Vec<_>>();
        write_wide_part(&path, &schema, &rows).unwrap();
        paths.push(path);
    }
    let query = parse_join(
        "SELECT count(*) FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id"
    ).unwrap().unwrap();
    let mut result = Vec::new();
    execute_multi_join(&query, &paths, &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(), "count\n20\n");

    let query = parse_join(
        "SELECT count(*) FROM a JOIN b ON a.id = b.id ANY JOIN c ON b.id = c.id"
    ).unwrap().unwrap();
    let mut result = Vec::new();
    execute_multi_join(&query, &paths, &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(), "count\n4\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn chained_count_rejects_non_inner_and_post_join_null_filters() {
    for sql in [
        "SELECT count(*) FROM a JOIN b ON a.id=b.id LEFT JOIN c ON b.id=c.id",
        "SELECT count(*) FROM a JOIN b ON a.id=b.id JOIN c ON b.id=c.id WHERE c.id IS NULL",
    ] {
        assert!(parse_join(sql).is_err(), "{sql}");
    }
}

#[test]
fn chained_count_reuses_string_key_probe_and_keeps_duplicate_multiplicities() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-string-chain-count-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let schema = [Column { name: "id".into(), data_type: "String".into() }];
    let mut paths = Vec::new();
    for (name, ids) in [
        ("a", &["a", "a", "b"][..]),
        ("b", &["a", "b"][..]),
        ("c", &["a", "a", "b"][..]),
    ] {
        let path = root.join(name);
        let rows = ids.iter().map(|id| vec![Some((*id).to_owned())]).collect::<Vec<_>>();
        write_wide_part(&path, &schema, &rows).unwrap();
        paths.push(path);
    }
    for (sql, expected) in [
        ("SELECT count(*) FROM a JOIN b ON a.id=b.id JOIN c ON b.id=c.id", 5),
        ("SELECT count(*) FROM a JOIN b ON a.id=b.id ANY JOIN c ON b.id=c.id", 3),
    ] {
        let query = parse_join(sql).unwrap().unwrap();
        let mut out = Vec::new();
        execute_multi_join(&query, &paths, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), format!("count\n{expected}\n"));
    }
    fs::remove_dir_all(root).unwrap();
}
