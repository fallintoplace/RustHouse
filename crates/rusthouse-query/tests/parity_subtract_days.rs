use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn ubtract_days_handles_calendar_boundaries() {
    for (sql, expected) in [
        ("SELECT subtractDays('2024-03-01', 1)", "2024-02-29"),
        ("SELECT subtractDays('2024-01-01', 1)", "2023-12-31"),
        ("SELECT subtractDays('2024-02-29 22:33:44', -1)", "2024-03-01 22:33:44"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT subtractDays(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn ubtract_days_rejects_unsupported_inputs() {
    for sql in [
        "SELECT subtractDays()",
        "SELECT subtractDays('2024-01-01')",
        "SELECT subtractDays('2023-02-29', 1)",
        "SELECT subtractDays('1900-01-01', 1)",
        "SELECT subtractDays('2024-01-01', 'x')",
        "SELECT subtractDays('2024-01-01', 1, 2)",
        "SELECT subtractDays('2024-01-01', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
