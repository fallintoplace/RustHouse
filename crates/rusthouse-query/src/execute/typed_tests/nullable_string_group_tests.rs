use crate::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn groups_nullable_text_with_borrowed_non_null_keys() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-null-text-group-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [Column {
        name: "category".into(),
        data_type: "Nullable(String)".into(),
    }];
    let rows = [
        vec![Some("x".into())],
        vec![None],
        vec![Some("x".into())],
        vec![Some("y".into())],
        vec![None],
    ];
    write_wide_part(&part, &columns, &rows).unwrap();
    let query = parse_select("SELECT category, count(*) FROM part GROUP BY category").unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&part), &mut output).unwrap();
    assert_eq!(output, b"category\tcount\nx\t2\n\\N\t2\ny\t1\n");
    std::fs::remove_dir_all(part).unwrap();
}
