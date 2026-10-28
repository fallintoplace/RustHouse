use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn subtract_minutes_roundtrips_across_boundaries() {
    for (sql, expected) in [
        ("SELECT subtractMinutes('2024-03-01 00:00:30', 1)", "2024-02-29 23:59:30"),
        ("SELECT subtractMinutes('2025-01-01 00:00:00', 1)", "2024-12-31 23:59:00"),
        ("SELECT subtractMinutes('2024-02-28 23:59:30', -1)", "2024-02-29 00:00:30"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT subtractMinutes(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn subtract_minutes_rejects_bad_arguments() {
    for sql in [
        "SELECT subtractMinutes()",
        "SELECT subtractMinutes('2024-01-01 00:00:00')",
        "SELECT subtractMinutes('2024-01-01', 1)",
        "SELECT subtractMinutes('2023-02-29 00:00:00', 1)",
        "SELECT subtractMinutes('1900-01-01 00:00:00', 1)",
        "SELECT subtractMinutes('2024-01-01 00:00:00', 'x')",
        "SELECT subtractMinutes('2024-01-01 00:00:00', 1, 2)",
        "SELECT subtractMinutes('2024-01-01 00:00:00', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
