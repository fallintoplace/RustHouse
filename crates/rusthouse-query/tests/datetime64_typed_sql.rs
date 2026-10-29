use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn typed_datetime64_sql_handles_fractional_boundaries_and_negative_timestamps() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-datetime64-query-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cols = [
        Column {name:"event".into(), data_type:"UInt8".into()},
        Column {name:"ts".into(), data_type:"DateTime64(3)".into()},
        Column {name:"processed".into(), data_type:"Nullable(DateTime64(3))".into()},
    ];
    let rows = [
        vec![Some("1".into()), Some("1969-12-31 23:59:59.250".into()), None],
        vec![Some("2".into()), Some("1970-01-01 00:00:00.001".into()),
            Some("1970-01-01 00:00:00.020".into())],
        vec![Some("3".into()), Some("1970-01-01 00:00:00.010".into()),
            Some("1969-12-31 23:59:59.250".into())],
    ];
    write_wide_part(&root, &cols, &rows).unwrap();

    let mut result = Vec::new();
    execute(&parse_select(
        "SELECT event, ts FROM part WHERE ts >= '1970-01-01 00:00:00.001' ORDER BY ts DESC"
    ).unwrap(), Some(&root), &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(),
        "event\tts\n3\t1970-01-01 00:00:00.010\n2\t1970-01-01 00:00:00.001\n");

    let mut result = Vec::new();
    execute(&parse_select(
        "SELECT processed FROM part WHERE processed IS NOT NULL ORDER BY processed"
    ).unwrap(), Some(&root), &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(),
        "processed\n1969-12-31 23:59:59.250\n1970-01-01 00:00:00.020\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_fractional_literals_rejected_before_scan() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-datetime64-invalid-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cols = [Column {name:"ts".into(), data_type:"DateTime64(3)".into()}];
    write_wide_part(&root, &cols, &[vec![Some("1970-01-01 00:00:00.000".into())]]).unwrap();
    let query = parse_select("SELECT ts FROM part WHERE ts = '1970-01-01 00:00:00.1234'").unwrap();
    assert!(execute(&query, Some(&root), &mut Vec::new()).is_err());
    fs::remove_dir_all(root).unwrap();
}
