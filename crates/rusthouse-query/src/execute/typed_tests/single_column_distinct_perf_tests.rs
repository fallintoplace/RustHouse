use crate::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn single_string_distinct_preserves_first_seen_order() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-single-distinct-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let columns = [Column {
        name: "word".into(),
        data_type: "String".into(),
    }];
    let rows = [
        vec![Some("alpha".into())],
        vec![Some("beta".into())],
        vec![Some("alpha".into())],
        vec![Some("gamma".into())],
        vec![Some("beta".into())],
    ];
    write_wide_part(&part, &columns, &rows).unwrap();
    let query = parse_select("SELECT DISTINCT word FROM part").unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&part), &mut output).unwrap();
    assert_eq!(output, b"word\nalpha\nbeta\ngamma\n");
    std::fs::remove_dir_all(part).unwrap();
}
