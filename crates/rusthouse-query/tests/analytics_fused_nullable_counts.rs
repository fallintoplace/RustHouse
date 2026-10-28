//! Multi-column nullable count queries only need validated null-map streams.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn fused_nullable_counts_match_full_semantics_with_repeated_columns() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-nullmap-multicount-{}", std::process::id()
    ));
    let schema = [
        Column { name: "id".into(), data_type: "UInt64".into() },
        Column { name: "a".into(), data_type: "Nullable(UInt64)".into() },
        Column { name: "b".into(), data_type: "Nullable(String)".into() },
    ];
    let rows = [
        vec![Some("1".into()), Some("10".into()), None],
        vec![Some("2".into()), None, Some("hello".into())],
        vec![Some("3".into()), Some("30".into()), Some("world".into())],
        vec![Some("4".into()), None, None],
    ];
    write_wide_part(&part, &schema, &rows).unwrap();
    let mut output = Vec::new();
    let sql = "SELECT count(*), count(a), count(b), count(id), count(a) FROM part";
    execute(&parse_select(sql).unwrap(), Some(&part), &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "count(*)\tcount(a)\tcount(b)\tcount(id)\tcount(a)\n4\t2\t2\t4\t2\n"
    );
    fs::remove_dir_all(part).unwrap();
}
