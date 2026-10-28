use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_bounded_clickhouse_constants() {
    assert_eq!(
        value("SELECT toStartOfMinute('2024-02-29 10:17:42')"),
        ConstantValue::Text("2024-02-29 10:17:00".into())
    );
    assert_eq!(
        value("SELECT toStartOfMinute('2024-02-29 10:17:00')"),
        ConstantValue::Text("2024-02-29 10:17:00".into())
    );
    assert_eq!(
        value("SELECT toStartOfMinute(NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_invalid_dates_and_arguments() {
    for sql in [
        "SELECT toStartOfMinute('2024-02-30 25:17:42')",
        "SELECT toStartOfMinute()",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
