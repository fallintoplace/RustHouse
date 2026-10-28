use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn exact_numeric_column_count_matches_part_rows_and_pagination() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-metadata-count-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = vec![Column { name: "id".into(), data_type: "UInt64".into() }];
    for (index, count) in [0usize, 3, 7].into_iter().enumerate() {
        let values = (0..count).map(|value| vec![Some(value.to_string())]).collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{index}_{index}_0")), &columns, &values).unwrap();
    }
    for (sql, expected) in [
        ("SELECT count(id) FROM part", "count(id)\n10\n"),
        ("SELECT count(id) FROM part LIMIT 0", "count(id)\n"),
        ("SELECT count(id) FROM part LIMIT 1 OFFSET 1", "count(id)\n"),
        ("SELECT count(id) FROM part WHERE id >= 3", "count(id)\n4\n"),
    ] {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
