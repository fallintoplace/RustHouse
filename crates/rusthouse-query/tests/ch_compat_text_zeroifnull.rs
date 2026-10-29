use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).expect(sql).projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn clickhouse_constant_compatibility() {
    assert_eq!(value("SELECT zeroIfNull(NULL)"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT zeroIfNull(4)"), ConstantValue::Integer(4));
    assert_eq!(value("SELECT zeroIfNull(0)"), ConstantValue::Integer(0));
}
#[test]
fn rejects_wrong_arguments() {
    assert!(parse_select("SELECT zeroIfNull()").is_err());
    assert!(parse_select("SELECT zeroIfNull('x')").is_err());
    assert!(parse_select("SELECT zeroIfNull(1, 2)").is_err());
}
