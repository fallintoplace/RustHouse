use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn subtract_months_clamps_month_end_days() {
    for (sql, expected) in [
        ("SELECT subtractMonths('2024-03-31', 1)", "2024-02-29"),
        ("SELECT subtractMonths('2023-03-31', 1)", "2023-02-28"),
        ("SELECT subtractMonths('2025-01-31', 2)", "2024-11-30"),
        ("SELECT subtractMonths('2024-03-01 12:30:59', 1)", "2024-02-01 12:30:59"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT subtractMonths(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn subtract_months_rejects_invalid_calendar_arithmetic() {
    for sql in [
        "SELECT subtractMonths()",
        "SELECT subtractMonths('2024-02-29')",
        "SELECT subtractMonths('2023-02-29', 1)",
        "SELECT subtractMonths('1900-01-01', 1)",
        "SELECT subtractMonths('2024-01-01', 'x')",
        "SELECT subtractMonths('2024-01-01', 1, 2)",
        "SELECT subtractMonths('2024-01-01', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
