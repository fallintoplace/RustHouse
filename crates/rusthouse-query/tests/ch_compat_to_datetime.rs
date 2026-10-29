use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).expect(sql).projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn formats_utc_values_and_propagates_null() {
    assert_eq!(value("SELECT toDateTime('2024-02-29 23:59:58')"), ConstantValue::Text("2024-02-29 23:59:58".into()));
    assert_eq!(value("SELECT toDateTime('2024-02-29')"), ConstantValue::Text("2024-02-29 00:00:00".into()));
    assert_eq!(value("SELECT toDateTime(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_date_and_arity() {
    assert!(parse_select("SELECT toDateTime()").is_err());
    assert!(parse_select("SELECT toDateTime('2024-02-30')").is_err());
    assert!(parse_select("SELECT toDateTime('2024-01-01', '2024-01-02')").is_err());
}
