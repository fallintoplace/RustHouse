use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn dd_weeks_handles_calendar_boundaries() {
    for (sql, expected) in [
        ("SELECT addWeeks('2024-02-22', 1)", "2024-02-29"),
        ("SELECT addWeeks('2023-12-25', 1)", "2024-01-01"),
        ("SELECT addWeeks('2024-03-07', -1)", "2024-02-29"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addWeeks(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn dd_weeks_rejects_unsupported_inputs() {
    for sql in [
        "SELECT addWeeks()",
        "SELECT addWeeks('2024-01-01')",
        "SELECT addWeeks('2023-02-29', 1)",
        "SELECT addWeeks('2299-12-31', 1)",
        "SELECT addWeeks('2024-01-01', 'x')",
        "SELECT addWeeks('2024-01-01', 1, 2)",
        "SELECT addWeeks('2024-01-01', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
