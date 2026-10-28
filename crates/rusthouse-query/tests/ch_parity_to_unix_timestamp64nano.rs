use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn converts_utc_timestamp_constants_with_fractional_precision() {
    for (sql, expected) in [
        ("SELECT toUnixTimestamp64Nano('2022-01-01 00:00:00.123456789')", 1_640_995_200_123_456_789),
        ("SELECT toUnixTimestamp64Nano('1970-01-01 00:00:00.000000001')", 1),
        ("SELECT toUnixTimestamp64Nano('2022-01-01T01:00:00+01:00')", 1_640_995_200_000_000_000),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toUnixTimestamp64Nano(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_fractional_datetime_literals() {
    for sql in [
        "SELECT toUnixTimestamp64Nano()",
        "SELECT toUnixTimestamp64Nano('2023-02-29 12:00:00')",
        "SELECT toUnixTimestamp64Nano('2022-01-01 00:00:00.1234567890')",
        "SELECT toUnixTimestamp64Nano('2022-01-01 00:00:00.x')",
        "SELECT toUnixTimestamp64Nano(123)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
