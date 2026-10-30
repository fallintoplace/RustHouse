use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn date32_predicates_ordering_and_nullable_projection_round_trip() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-date32-sql-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cols = [
        Column { name:"id".into(), data_type:"UInt8".into() },
        Column { name:"shipday".into(), data_type:"Date32".into() },
        Column { name:"closed".into(), data_type:"Nullable(Date32)".into() },
    ];
    let rows = [
        vec![Some("1".into()), Some("1969-12-31".into()), None],
        vec![Some("2".into()), Some("2000-02-29".into()), Some("1970-01-01".into())],
        vec![Some("3".into()), Some("1970-01-01".into()), Some("1969-12-31".into())],
    ];
    write_wide_part(&root, &cols, &rows).unwrap();
    let mut out = Vec::new();
    let query = parse_select(
        "SELECT id, shipday FROM part WHERE shipday >= '1970-01-01' ORDER BY shipday DESC"
    ).unwrap();
    execute(&query, Some(&root), &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(),
        "id\tshipday\n2\t2000-02-29\n3\t1970-01-01\n");

    let mut out = Vec::new();
    let query = parse_select(
        "SELECT closed FROM part WHERE closed IS NOT NULL ORDER BY closed ASC"
    ).unwrap();
    execute(&query, Some(&root), &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(),
        "closed\n1969-12-31\n1970-01-01\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn date32_rejects_invalid_calendar_day_literal() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-date32-invalid-{}-{}", std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cols = [Column { name:"shipday".into(), data_type:"Date32".into() }];
    write_wide_part(&dir, &cols, &[vec![Some("1970-01-01".into())]]).unwrap();
    let query = parse_select("SELECT shipday FROM part WHERE shipday = '1900-02-29'").unwrap();
    assert!(execute(&query, Some(&dir), &mut Vec::new()).is_err());
    fs::remove_dir_all(dir).unwrap();
}
