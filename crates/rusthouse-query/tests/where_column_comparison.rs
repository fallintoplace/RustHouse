use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn compares_two_date_and_nullable_number_columns_in_where() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-compare-columns-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column { name: "commit_day".into(), data_type: "Date".into() },
        Column { name: "receipt_day".into(), data_type: "Date".into() },
        Column { name: "left_value".into(), data_type: "Nullable(Int64)".into() },
        Column { name: "right_value".into(), data_type: "Int64".into() },
    ];
    let rows = [
        vec![Some("1994-01-01".into()), Some("1994-01-03".into()), Some("5".into()), Some("7".into())],
        vec![Some("1994-01-03".into()), Some("1994-01-01".into()), Some("10".into()), Some("9".into())],
        vec![Some("1994-01-05".into()), Some("1994-01-06".into()), None, Some("10".into())],
        vec![Some("1994-01-07".into()), Some("1994-01-08".into()), Some("10".into()), Some("10".into())],
    ];
    write_wide_part(&part, &columns, &rows).unwrap();
    for (sql, expected) in [
        ("SELECT count(*) FROM part WHERE commit_day < receipt_day", "3"),
        ("SELECT count(*) FROM part WHERE left_value < right_value", "1"),
        ("SELECT count(*) FROM part WHERE left_value >= right_value", "2"),
        ("SELECT count(*) FROM part WHERE NOT (left_value < right_value)", "2"),
    ] {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&part), &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap().lines().last(), Some(expected), "{sql}");
    }
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn rejects_unknown_or_incompatible_column_comparisons() {
    let parsed = parse_select("SELECT count(*) FROM part WHERE commit_day < receipt_day").unwrap();
    assert_eq!(parsed.predicates.len(), 1);
    assert!(parse_select("SELECT count(*) FROM part WHERE value <").is_err());
}
