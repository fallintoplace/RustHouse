//! Fused metadata COUNT over wide immutable parts without scanning streams.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn with_part<F: FnOnce(&std::path::Path)>(test: F) {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-metadata-multi-count-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let schema = [
        Column { name: "id".into(), data_type: "UInt64".into() },
        Column { name: "label".into(), data_type: "String".into() },
        Column { name: "nullable".into(), data_type: "Nullable(UInt64)".into() },
    ];
    let rows = [
        vec![Some("1".into()), Some("a".into()), Some("3".into())],
        vec![Some("2".into()), Some("b".into()), None],
        vec![Some("3".into()), Some("c".into()), Some("4".into())],
    ];
    write_wide_part(&dir, &schema, &rows).unwrap();
    test(&dir);
    fs::remove_dir_all(dir).unwrap();
}

fn run(path: &std::path::Path, sql: &str) -> String {
    let mut out = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(path), &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn mixed_non_nullable_columns_count_from_verified_part_rows() {
    with_part(|path| {
        assert_eq!(
            run(path, "SELECT count(*), count(id), count(label) FROM part"),
            "count(*)\tcount(id)\tcount(label)\n3\t3\t3\n"
        );
        assert_eq!(
            run(path, "SELECT count(id), count(*) FROM part LIMIT 0"),
            "count(id)\tcount(*)\n"
        );
    });
}
#[test]
fn nullable_and_filtered_columns_keep_row_scan_semantics() {
    with_part(|path| {
        assert_eq!(
            run(path, "SELECT count(*), count(nullable) FROM part"),
            "count(*)\tcount(nullable)\n3\t2\n"
        );
        assert_eq!(
            run(path, "SELECT count(*), count(id) FROM part WHERE id >= 2"),
            "count(*)\tcount(id)\n2\t2\n"
        );
    });
}
