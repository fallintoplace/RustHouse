use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn handles_date_boundaries_and_nulls() {
    assert_eq!(value("SELECT toStartOfWeek('2023-04-21')"), ConstantValue::Text("2023-04-16".into()));
    assert_eq!(value("SELECT toStartOfWeek('2023-04-21', 1)"), ConstantValue::Text("2023-04-17".into()));
    assert_eq!(value("SELECT toStartOfWeek(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_date_and_arity() {
    assert!(parse_select("SELECT toStartOfWeek('2024-02-30')").is_err());
    assert!(parse_select("SELECT toStartOfWeek()").is_err());
    assert!(parse_select("SELECT toStartOfWeek('2024-01-01', 2)").is_err());
}
