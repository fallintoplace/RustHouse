use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn converts_bounded_utc_date_constants() {
    for (sql, expected) in [
        ("SELECT toUnixTimestamp('1970-01-01')", ConstantValue::Integer(0)),
        ("SELECT toUnixTimestamp('2024-03-01T01:00:00+01:00')", ConstantValue::Integer(1709251200)),
        ("SELECT toUnixTimestamp('2022-01-01 00:00:00')", ConstantValue::Integer(1640995200)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT toUnixTimestamp(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_or_out_of_range_inputs() {
    for sql in [
        "SELECT toUnixTimestamp()",
        "SELECT toUnixTimestamp('2023-02-29')",
        "SELECT toUnixTimestamp('nonsense')",
        "SELECT toUnixTimestamp(5)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
