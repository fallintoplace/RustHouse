use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_inner_count_scans_last_bucket_without_retaining_join_product() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-chain-count-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let schema = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    let mut paths = Vec::new();
    for (name, n) in [("a", 2), ("b", 2), ("c", 5)] {
        let path = root.join(name);
        let rows = (0..n).map(|_| vec![Some("7".into())]).collect::<Vec<_>>();
        write_wide_part(&path, &schema, &rows).unwrap();
        paths.push(path);
    }
    let query = parse_join("SELECT count(*) FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id")
        .unwrap()
        .unwrap();
    let mut result = Vec::new();
    execute_multi_join(&query, &paths, &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(), "count\n20\n");

    let query =
        parse_join("SELECT count(*) FROM a JOIN b ON a.id = b.id ANY JOIN c ON b.id = c.id")
            .unwrap()
            .unwrap();
    let mut result = Vec::new();
    execute_multi_join(&query, &paths, &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(), "count\n4\n");
    let missing = root.join("missing");
    write_wide_part(&missing, &schema, &[vec![Some("8".into())]]).unwrap();
    let query = parse_join("SELECT count(*) FROM a JOIN b ON a.id=b.id LEFT JOIN c ON b.id=c.id")
        .unwrap()
        .unwrap();
    let mut result = Vec::new();
    execute_multi_join(
        &query,
        &[paths[0].clone(), paths[1].clone(), missing],
        &mut result,
    )
    .unwrap();
    assert_eq!(String::from_utf8(result).unwrap(), "count\n4\n");
    let query = parse_join(
        "SELECT count(*) FROM a JOIN b ON a.id=b.id JOIN c ON b.id=c.id WHERE c.id IS NULL",
    )
    .unwrap()
    .unwrap();
    let mut result = Vec::new();
    execute_multi_join(&query, &paths, &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(), "count\n0\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unqualified_join_keys_resolve_when_unique_and_reject_ambiguity() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-unqualified-join-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let left = root.join("lefts");
    let right = root.join("rights");
    let duplicate = root.join("duplicate");
    write_wide_part(
        &left,
        &[Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        }],
        &[vec![Some("1".into())], vec![Some("2".into())]],
    )
    .unwrap();
    write_wide_part(
        &right,
        &[Column {
            name: "right_id".into(),
            data_type: "UInt64".into(),
        }],
        &[vec![Some("1".into())]],
    )
    .unwrap();
    write_wide_part(
        &duplicate,
        &[Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        }],
        &[vec![Some("1".into())]],
    )
    .unwrap();

    let query = parse_join("SELECT l.id FROM lefts l JOIN rights r ON right_id = id ORDER BY l.id")
        .unwrap()
        .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[left.clone(), right], &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "id\n1\n");

    let query = parse_join("SELECT l.id FROM lefts l JOIN duplicate d ON id = d.id")
        .unwrap()
        .unwrap();
    let error = execute_multi_join(&query, &[left, duplicate], &mut Vec::new()).unwrap_err();
    assert!(error.to_string().contains("ambiguous JOIN column: id"));

    fs::remove_dir_all(root).unwrap();
}
