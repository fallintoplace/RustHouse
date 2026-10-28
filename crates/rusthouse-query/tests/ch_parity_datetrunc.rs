use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn handles_bounded_calendar_and_null_values() {
    assert_eq!(
        value("SELECT dateTrunc('second', '2024-02-29 23:59:59')"),
        ConstantValue::Text("2024-02-29 23:59:59".into())
    );
    assert_eq!(
        value("SELECT dateTrunc('minute', '2024-02-29 23:59:59')"),
        ConstantValue::Text("2024-02-29 23:59:00".into())
    );
    assert_eq!(
        value("SELECT dateTrunc('hour', '2024-02-29 23:59:59')"),
        ConstantValue::Text("2024-02-29 23:00:00".into())
    );
    assert_eq!(
        value("SELECT dateTrunc('day', '2024-02-29 23:59:59')"),
        ConstantValue::Text("2024-02-29 00:00:00".into())
    );
    assert_eq!(
        value("SELECT dateTrunc(NULL, NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_bad_calendar_types_and_ranges() {
    for sql in [
        "SELECT dateTrunc()",
        "SELECT dateTrunc('week', '2024-02-29 23:59:59')",
        "SELECT dateTrunc('hour', 'bad')",
        "SELECT dateTrunc('hour', '2024-02-29 23:59:59', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
