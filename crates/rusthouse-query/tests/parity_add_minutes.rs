use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn add_minutes_roundtrips_across_boundaries() {
    for (sql, expected) in [
        ("SELECT addMinutes('2024-02-28 23:59:30', 1)", "2024-02-29 00:00:30"),
        ("SELECT addMinutes('2024-12-31 23:59:59', 1)", "2025-01-01 00:00:59"),
        ("SELECT addMinutes('2024-03-01 00:00:30', -1)", "2024-02-29 23:59:30"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addMinutes(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn add_minutes_rejects_bad_arguments() {
    for sql in [
        "SELECT addMinutes()",
        "SELECT addMinutes('2024-01-01 00:00:00')",
        "SELECT addMinutes('2024-01-01', 1)",
        "SELECT addMinutes('2023-02-29 00:00:00', 1)",
        "SELECT addMinutes('2299-12-31 23:59:59', 1)",
        "SELECT addMinutes('2024-01-01 00:00:00', 'x')",
        "SELECT addMinutes('2024-01-01 00:00:00', 1, 2)",
        "SELECT addMinutes('2024-01-01 00:00:00', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
