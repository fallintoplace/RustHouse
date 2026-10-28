use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn add_months_clamps_month_end_days() {
    for (sql, expected) in [
        ("SELECT addMonths('2024-01-31', 1)", "2024-02-29"),
        ("SELECT addMonths('2023-01-31', 1)", "2023-02-28"),
        ("SELECT addMonths('2024-12-31', 2)", "2025-02-28"),
        ("SELECT addMonths('2024-01-01 12:30:59', 1)", "2024-02-01 12:30:59"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addMonths(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn add_months_rejects_invalid_calendar_arithmetic() {
    for sql in [
        "SELECT addMonths()",
        "SELECT addMonths('2024-02-29')",
        "SELECT addMonths('2023-02-29', 1)",
        "SELECT addMonths('2299-12-31', 1)",
        "SELECT addMonths('2024-01-01', 'x')",
        "SELECT addMonths('2024-01-01', 1, 2)",
        "SELECT addMonths('2024-01-01', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
