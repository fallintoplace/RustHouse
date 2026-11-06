use crate::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn bounded_sort_keeps_earlier_ties_and_skips_losing_rows() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-typed-topk-prune-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column {
            name: "note".into(),
            data_type: "String".into(),
        },
        Column {
            name: "rank".into(),
            data_type: "UInt64".into(),
        },
    ];
    let mut rows = vec![
        vec![Some("first".into()), Some("1".into())],
        vec![Some("second".into()), Some("1".into())],
    ];
    rows.extend((0..300).map(|index| vec![Some(format!("loser-{index}")), Some("99".into())]));
    write_wide_part(&path, &columns, &rows).unwrap();
    let query = parse_select("SELECT note FROM part ORDER BY rank LIMIT 2").unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&path), &mut output).unwrap();
    assert_eq!(output, b"note\nfirst\nsecond\n");
    std::fs::remove_dir_all(path).unwrap();
}
