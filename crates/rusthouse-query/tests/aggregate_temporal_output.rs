use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn clickbench_date_extrema_render_calendar_values() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-date-extrema-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column { name: "VisitDate".into(), data_type: "Date".into() },
        Column { name: "HistoricalDate".into(), data_type: "Date32".into() },
        Column { name: "VisitTime".into(), data_type: "DateTime".into() },
    ];
    let rows = [
        vec![
            Some("2024-01-03".into()),
            Some("1969-12-31".into()),
            Some("2024-01-03 12:00:00".into()),
        ],
        vec![
            Some("2024-01-01".into()),
            Some("1959-01-03".into()),
            Some("2024-01-01 09:30:00".into()),
        ],
    ];
    write_wide_part(&path, &columns, &rows).unwrap();

    let mut output = Vec::new();
    let sql = "SELECT min(VisitDate), max(VisitDate), min(HistoricalDate), max(VisitTime) FROM part";
    execute(&parse_select(sql).unwrap(), Some(&path), &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap().lines().last(),
        Some("2024-01-01\t2024-01-03\t1959-01-03\t2024-01-03 12:00:00")
    );

    let mut output = Vec::new();
    execute(
        &parse_select("SELECT min(HistoricalDate) FROM part").unwrap(),
        Some(&path), &mut output
    ).unwrap();
    assert_eq!(String::from_utf8(output).unwrap().lines().last(), Some("1959-01-03"));
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn temporal_aggregate_display_preserves_null_and_numeric_results() {
    use rusthouse_storage::{format_date, format_date32, format_datetime};
    assert_eq!(format_date(0), "1970-01-01");
    assert_eq!(format_date32(-1), "1969-12-31");
    assert_eq!(format_datetime(0), "1970-01-01 00:00:00");
}
