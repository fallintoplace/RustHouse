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
        value("SELECT toStartOfISOYear('2023-04-21')"),
        ConstantValue::Text("2023-01-02".into())
    );
    assert_eq!(
        value("SELECT toStartOfISOYear('2021-01-01')"),
        ConstantValue::Text("2019-12-30".into())
    );
    assert_eq!(
        value("SELECT toStartOfISOYear(NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_invalid_dates_and_arguments() {
    for sql in [
        "SELECT toStartOfISOYear('bad')",
        "SELECT toStartOfISOYear()",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
