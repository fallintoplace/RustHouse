use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn converts_bounded_utc_date_constants() {
    for (sql, expected) in [
        ("SELECT toDateTime('2024-02-29')", ConstantValue::Text("2024-02-29 00:00:00".into())),
        ("SELECT toDateTime('2024-02-29 23:59:59')", ConstantValue::Text("2024-02-29 23:59:59".into())),
        ("SELECT toDateTime('2024-03-01T00:00:00+01:00')", ConstantValue::Text("2024-02-29 23:00:00".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT toDateTime(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_or_out_of_range_inputs() {
    for sql in [
        "SELECT toDateTime()",
        "SELECT toDateTime('2023-02-29')",
        "SELECT toDateTime('nonsense')",
        "SELECT toDateTime(5)",
        "SELECT toDateTime('2149-06-06')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
