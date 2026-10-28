use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn dd_hours_handles_calendar_boundaries() {
    for (sql, expected) in [
        ("SELECT addHours('2024-02-28 23:15:00', 2)", "2024-02-29 01:15:00"),
        ("SELECT addHours('2024-12-31 23:00:00', 1)", "2025-01-01 00:00:00"),
        ("SELECT addHours('2024-03-01 00:00:00', -1)", "2024-02-29 23:00:00"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addHours(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn dd_hours_rejects_unsupported_inputs() {
    for sql in [
        "SELECT addHours()",
        "SELECT addHours('2024-01-01')",
        "SELECT addHours('2023-02-29', 1)",
        "SELECT addHours('2299-12-31 23:59:59', 1)",
        "SELECT addHours('2024-01-01', 'x')",
        "SELECT addHours('2024-01-01', 1, 2)",
        "SELECT addHours('2024-01-01', 1) FROM numbers(1)",
        "SELECT addHours('2024-01-01', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
