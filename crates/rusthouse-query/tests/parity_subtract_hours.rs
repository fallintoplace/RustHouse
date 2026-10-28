use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn subtract_hours_roundtrips_across_boundaries() {
    for (sql, expected) in [
        ("SELECT subtractHours('2024-03-01 00:15:00', 1)", "2024-02-29 23:15:00"),
        ("SELECT subtractHours('2025-01-01 00:00:00', 1)", "2024-12-31 23:00:00"),
        ("SELECT subtractHours('2024-02-28 23:00:00', -1)", "2024-02-29 00:00:00"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT subtractHours(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn subtract_hours_rejects_bad_arguments() {
    for sql in [
        "SELECT subtractHours()",
        "SELECT subtractHours('2024-01-01 00:00:00')",
        "SELECT subtractHours('2024-01-01', 1)",
        "SELECT subtractHours('2023-02-29 00:00:00', 1)",
        "SELECT subtractHours('1900-01-01 00:00:00', 1)",
        "SELECT subtractHours('2024-01-01 00:00:00', 'x')",
        "SELECT subtractHours('2024-01-01 00:00:00', 1, 2)",
        "SELECT subtractHours('2024-01-01 00:00:00', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
