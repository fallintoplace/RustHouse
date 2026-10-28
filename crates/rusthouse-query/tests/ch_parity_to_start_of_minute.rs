use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn minute_bucket_uses_utc_and_keeps_calendar_boundaries() {
    for (sql, expected) in [
        ("SELECT toStartOfMinute('2024-02-29 23:59:58')", "2024-02-29 23:59:00"),
        ("SELECT toStartOfMinute('2024-03-01T00:00:30+01:00')", "2024-02-29 23:00:00"),
        ("SELECT toStartOfMinute('1970-01-01 00:00:59')", "1970-01-01 00:00:00"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toStartOfMinute(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_minute_inputs() {
    for sql in [
        "SELECT toStartOfMinute()",
        "SELECT toStartOfMinute('2023-02-29 00:00:00')",
        "SELECT toStartOfMinute('2024-01-01 24:00:00')",
        "SELECT toStartOfMinute(5)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
