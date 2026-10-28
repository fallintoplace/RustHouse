use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn to_start_of_five_minutes_truncates_utc_timestamps() {
    for (sql, expected) in [
        ("SELECT toStartOfFiveMinutes('2024-02-29 12:34:59')", "2024-02-29 12:30:00"),
        ("SELECT toStartOfFiveMinutes('2024-02-29 12:35:00')", "2024-02-29 12:35:00"),
        ("SELECT toStartOfFiveMinutes('2024-12-31 23:59:59')", "2024-12-31 23:55:00"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT toStartOfFiveMinutes(NULL)"), ConstantValue::Null);
}

#[test]
fn to_start_of_five_minutes_checks_datetime_and_arity() {
    for sql in [
        "SELECT toStartOfFiveMinutes()",
        "SELECT toStartOfFiveMinutes('2024-02-29')",
        "SELECT toStartOfFiveMinutes('2024-02-30 12:30:00')",
        "SELECT toStartOfFiveMinutes('2024-02-29 24:00:00')",
        "SELECT toStartOfFiveMinutes('2024-02-29 12:30:00', 2)",
        "SELECT toStartOfFiveMinutes('2024-02-29 12:30:00') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
