use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn typed_numeric_aggregate_skips_nulls_and_filters_strings() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-typed-numeric-fast-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let columns = [
        Column { name: "metric".into(), data_type: "Nullable(UInt64)".into() },
        Column { name: "category".into(), data_type: "String".into() },
    ];
    let rows = [
        vec![Some("3".into()), Some("keep".into())],
        vec![None, Some("keep".into())],
        vec![Some("5".into()), Some("skip".into())],
        vec![Some("4".into()), Some("keep".into())],
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    let mut output = Vec::new();
    execute(
        &parse_select("SELECT sum(metric) FROM part WHERE category = 'keep'").unwrap(),
        Some(&root),
        &mut output,
    ).unwrap();
    assert_eq!(output, b"sum(metric)\n7\n");
    std::fs::remove_dir_all(root).unwrap();
}
