use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn to_start_of_ten_minutes_uses_fixed_utc_buckets() {
    for (sql, expected) in [
        ("SELECT toStartOfTenMinutes('2024-02-29 12:39:59')", "2024-02-29 12:30:00"),
        ("SELECT toStartOfTenMinutes('2024-02-29 12:40:00')", "2024-02-29 12:40:00"),
        ("SELECT toStartOfTenMinutes('2024-12-31 23:59:59')", "2024-12-31 23:50:00"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toStartOfTenMinutes(NULL)"), ConstantValue::Null);
}

#[test]
fn to_start_of_ten_minutes_rejects_unsupported_parameters() {
    for sql in [
        "SELECT toStartOfTenMinutes()",
        "SELECT toStartOfTenMinutes('2024-02-29')",
        "SELECT toStartOfTenMinutes('2023-02-29 12:00:00')",
        "SELECT toStartOfTenMinutes('2024-02-29 24:00:00')",
        "SELECT toStartOfTenMinutes('2024-02-29 12:30:00', 1)",
        "SELECT toStartOfTenMinutes('2024-02-29 12:30:00') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
