use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).expect(sql).projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn supports_utc_values_and_null() {
    assert_eq!(value("SELECT toMinute('2024-02-29 23:59:58')"), ConstantValue::Integer(59));
    assert_eq!(value("SELECT toMinute('2024-02-29')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toMinute(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_bad_arguments() {
    assert!(parse_select("SELECT toMinute()").is_err());
    assert!(parse_select("SELECT toMinute('2024-13-01')").is_err());
    assert!(parse_select("SELECT toMinute('2024-02-29', '2024-03-01')").is_err());
}
