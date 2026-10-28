use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn add_seconds_crosses_day_boundaries() {
    for (sql, expected) in [
        ("SELECT addSeconds('2024-02-28 23:59:59', 1)", "2024-02-29 00:00:00"),
        ("SELECT addSeconds('2024-12-31 23:59:59', 1)", "2025-01-01 00:00:00"),
        ("SELECT addSeconds('2024-03-01 00:00:00', -1)", "2024-02-29 23:59:59"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT addSeconds(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn add_seconds_validates_inputs() {
    for sql in [
        "SELECT addSeconds()",
        "SELECT addSeconds('2024-02-29 00:00:00')",
        "SELECT addSeconds('2024-02-29', 1)",
        "SELECT addSeconds('2023-02-29 00:00:00', 1)",
        "SELECT addSeconds('2299-12-31 23:59:59', 1)",
        "SELECT addSeconds('2024-02-29 00:00:00', 'bad')",
        "SELECT addSeconds('2024-02-29 00:00:00', 1, 2)",
        "SELECT addSeconds('2024-02-29 00:00:00', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
