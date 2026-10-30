use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn having_accepts_selected_metric_alias_and_preserves_aggregation() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-having-alias-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column { name: "region".into(), data_type: "String".into() },
        Column { name: "value".into(), data_type: "UInt64".into() },
    ];
    let rows = [
        vec![Some("east".into()), Some("20".into())],
        vec![Some("east".into()), Some("30".into())],
        vec![Some("west".into()), Some("3".into())],
    ];
    write_wide_part(&path, &columns, &rows).unwrap();
    let sql = "SELECT region, sum(value) AS total, count(value) AS samples FROM part GROUP BY region HAVING total > 40 ORDER BY region";
    let mut out = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&path), &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "region\ttotal\tsamples\neast\t50\t2\n");
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn rejects_unknown_and_unselected_metric_aliases() {
    for sql in [
        "SELECT k, sum(v) AS total, count(v) AS n FROM part GROUP BY k HAVING absent > 10",
        "SELECT k, sum(v) AS total, count(v) AS n FROM part GROUP BY k HAVING max(v) > 10",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
