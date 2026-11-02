use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn grouped_date_extrema_keep_calendar_output_on_all_group_paths() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-grouped-date-extrema-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column {
            name: "grp".into(),
            data_type: "String".into(),
        },
        Column {
            name: "segment".into(),
            data_type: "UInt8".into(),
        },
        Column {
            name: "visit".into(),
            data_type: "Date".into(),
        },
        Column {
            name: "historic".into(),
            data_type: "Date32".into(),
        },
        Column {
            name: "event_time".into(),
            data_type: "DateTime".into(),
        },
    ];
    let rows = [
        vec![
            Some("one".into()),
            Some("1".into()),
            Some("2024-01-01".into()),
            Some("1969-12-31".into()),
            Some("2024-01-01 10:00:00".into()),
        ],
        vec![
            Some("one".into()),
            Some("1".into()),
            Some("2024-01-03".into()),
            Some("1969-12-30".into()),
            Some("2024-01-03 14:00:00".into()),
        ],
        vec![
            Some("two".into()),
            Some("2".into()),
            Some("2024-01-09".into()),
            Some("1970-01-02".into()),
            Some("2024-01-09 23:59:59".into()),
        ],
    ];
    write_wide_part(&path, &columns, &rows).unwrap();
    for (sql, expected) in [
        (
            "SELECT grp, MIN(visit) FROM part GROUP BY grp ORDER BY grp",
            "one\t2024-01-01",
        ),
        (
            "SELECT grp, MIN(visit), MAX(historic) FROM part GROUP BY grp ORDER BY grp",
            "one\t2024-01-01\t1969-12-31",
        ),
        (
            "SELECT grp, segment, MIN(visit), MAX(historic) FROM part GROUP BY grp, segment ORDER BY grp, segment",
            "one\t1\t2024-01-01\t1969-12-31",
        ),
        (
            "SELECT grp, MAX(event_time) FROM part GROUP BY grp ORDER BY grp",
            "one\t2024-01-03 14:00:00",
        ),
        (
            "SELECT grp, MIN(visit), MAX(event_time) FROM part GROUP BY grp ORDER BY grp",
            "one\t2024-01-01\t2024-01-03 14:00:00",
        ),
    ] {
        let mut out = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&path), &mut out).unwrap();
        let result = String::from_utf8(out).unwrap();
        assert!(
            result.lines().any(|line| line == expected),
            "{sql}: {result}"
        );
    }
    fs::remove_dir_all(path).unwrap();
}
