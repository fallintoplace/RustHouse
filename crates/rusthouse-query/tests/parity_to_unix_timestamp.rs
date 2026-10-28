use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn to_unix_timestamp_handles_utc_boundaries() {
    for (sql, expected) in [
        ("SELECT toUnixTimestamp('1970-01-01 00:00:00')", ConstantValue::Integer(0)),
        ("SELECT toUnixTimestamp('1970-01-02 00:00:01')", ConstantValue::Integer(86401)),
        ("SELECT toUnixTimestamp('2000-01-01 00:00:00')", ConstantValue::Integer(946684800)),
        ("SELECT toUnixTimestamp('2106-02-07 06:28:15')", ConstantValue::Integer(4294967295)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT toUnixTimestamp(NULL)"), ConstantValue::Null);
}

#[test]
fn to_unix_timestamp_rejects_bad_arguments() {
    for sql in [
        "SELECT toUnixTimestamp()",
        "SELECT toUnixTimestamp('1969-12-31 23:59:59')",
        "SELECT toUnixTimestamp('2106-02-07 06:28:16')",
        "SELECT toUnixTimestamp('2023-02-29')",
        "SELECT toUnixTimestamp('2024-02-29 12:30:00', 'Europe/Berlin')",
        "SELECT toUnixTimestamp('2024-02-29') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
