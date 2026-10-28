//! Filtered nullable integer medians over large real MergeTree parts.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn filtered_nullable_median_is_exact_across_more_than_100k_fact_rows() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-filtered-nullable-radix-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let columns = [
        Column { name: "value".into(), data_type: "Nullable(UInt64)".into() },
        Column { name: "id".into(), data_type: "UInt64".into() },
    ];
    for (part, begin, end) in [(1, 0_u64, 60_001_u64), (2, 60_001, 120_002)] {
        let rows = (begin..end).map(|id| vec![
            (id % 2 == 0).then(|| id.to_string()),
            Some(id.to_string()),
        ]).collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{part}_{part}_0")), &columns, &rows).unwrap();
    }
    let run = |sql: &str| {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    };
    assert_eq!(
        run("SELECT median(value) FROM part WHERE id >= 20 AND id < 119980"),
        "median(value)\n59999\n"
    );
    assert_eq!(
        run("SELECT median(value) FROM part WHERE id > 120100"),
        "median(value)\nNULL\n"
    );
    fs::remove_dir_all(root).unwrap();
}
