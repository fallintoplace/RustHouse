use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn subtract_years_preserves_gregorian_calendar_dates() {
    for (sql, expected) in [
        ("SELECT subtractYears('2024-02-29', 1)", "2023-02-28"),
        ("SELECT subtractYears('2024-02-29', 4)", "2020-02-29"),
        ("SELECT subtractYears('2025-01-01', 1)", "2024-01-01"),
        ("SELECT subtractYears('2024-06-15 15:40:03', 1)", "2023-06-15 15:40:03"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT subtractYears(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn subtract_years_checks_date_ranges_and_inputs() {
    for sql in [
        "SELECT subtractYears()",
        "SELECT subtractYears('2024-02-29')",
        "SELECT subtractYears('2023-02-29', 1)",
        "SELECT subtractYears('1900-01-01', 1)",
        "SELECT subtractYears('2024-01-01', 'x')",
        "SELECT subtractYears('2024-01-01', 1, 2)",
        "SELECT subtractYears('2024-01-01', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
