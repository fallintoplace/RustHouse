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
        ("SELECT fromUnixTimestamp(0)", ConstantValue::Text("1970-01-01 00:00:00".into())),
        ("SELECT fromUnixTimestamp(1640995200)", ConstantValue::Text("2022-01-01 00:00:00".into())),
        ("SELECT fromUnixTimestamp(1709251200)", ConstantValue::Text("2024-03-01 00:00:00".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT fromUnixTimestamp(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_or_out_of_range_inputs() {
    for sql in [
        "SELECT fromUnixTimestamp()",
        "SELECT fromUnixTimestamp(-1)",
        "SELECT fromUnixTimestamp(4294967296)",
        "SELECT fromUnixTimestamp('2024-01-01')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
