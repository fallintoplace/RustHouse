use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn to_start_of_week_handles_utc_boundaries() {
    for (sql, expected) in [
        ("SELECT toStartOfWeek('2024-02-29')", ConstantValue::Text("2024-02-25".into())),
        ("SELECT toStartOfWeek('2024-02-29', 1)", ConstantValue::Text("2024-02-26".into())),
        ("SELECT toStartOfWeek('2024-03-03')", ConstantValue::Text("2024-03-03".into())),
        ("SELECT toStartOfWeek('2024-03-04', 1)", ConstantValue::Text("2024-03-04".into())),
        ("SELECT toStartOfWeek('2024-02-29 22:15:42', 1)", ConstantValue::Text("2024-02-26".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT toStartOfWeek(NULL)"), ConstantValue::Null);
}

#[test]
fn to_start_of_week_rejects_bad_arguments() {
    for sql in [
        "SELECT toStartOfWeek()",
        "SELECT toStartOfWeek('2024-02-29', 2)",
        "SELECT toStartOfWeek('2024-02-29', -1)",
        "SELECT toStartOfWeek('1899-12-31')",
        "SELECT toStartOfWeek('2023-02-29')",
        "SELECT toStartOfWeek('2024-02-29', 1, 'UTC')",
        "SELECT toStartOfWeek('2024-02-29') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
