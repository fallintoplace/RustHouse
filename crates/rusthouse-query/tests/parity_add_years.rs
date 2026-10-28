use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn add_years_preserves_gregorian_calendar_dates() {
    for (sql, expected) in [
        ("SELECT addYears('2024-02-29', 1)", "2025-02-28"),
        ("SELECT addYears('2020-02-29', 4)", "2024-02-29"),
        ("SELECT addYears('2024-12-31', 1)", "2025-12-31"),
        ("SELECT addYears('2024-06-15 15:40:03', 1)", "2025-06-15 15:40:03"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addYears(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn add_years_checks_date_ranges_and_inputs() {
    for sql in [
        "SELECT addYears()",
        "SELECT addYears('2024-02-29')",
        "SELECT addYears('2023-02-29', 1)",
        "SELECT addYears('2299-12-31', 1)",
        "SELECT addYears('2024-01-01', 'x')",
        "SELECT addYears('2024-01-01', 1, 2)",
        "SELECT addYears('2024-01-01', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
