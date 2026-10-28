use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn rounds_utc_datetime_constants_to_interval_start() {
    for (sql, expected) in [
        ("SELECT toStartOfTenMinutes('2024-02-29 23:59:58')", "2024-02-29 23:50:00"),
        ("SELECT toStartOfTenMinutes('2024-03-01T00:02:30+01:00')", "2024-02-29 23:00:00"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toStartOfTenMinutes(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_datetime_constants() {
    for sql in [
        "SELECT toStartOfTenMinutes()",
        "SELECT toStartOfTenMinutes('2023-02-29 00:00:00')",
        "SELECT toStartOfTenMinutes('2024-01-01 24:00:00')",
        "SELECT toStartOfTenMinutes(5)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
