use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_exact_signed_unix_seconds_from_utc_dates() {
    for (sql, expected) in [
        ("SELECT toUnixTimestamp64Second('1970-01-01 00:00:00')", 0),
        ("SELECT toUnixTimestamp64Second('1969-12-31 23:59:59')", -1),
        ("SELECT toUnixTimestamp64Second('2024-01-01 00:00:00')", 1_704_067_200),
        ("SELECT toUnixTimestamp64Second('2000-02-29')", 951_782_400),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT toUnixTimestamp64Second(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_unsupported_arguments() {
    for sql in [
        "SELECT toUnixTimestamp64Second()",
        "SELECT toUnixTimestamp64Second('2024-02-30')",
        "SELECT toUnixTimestamp64Second('2024-01-01', 'UTC')",
        "SELECT toUnixTimestamp64Second(123)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
