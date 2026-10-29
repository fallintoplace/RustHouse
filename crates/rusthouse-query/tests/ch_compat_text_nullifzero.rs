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
    assert_eq!(value("SELECT nullIfZero(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT nullIfZero(0)"), ConstantValue::Null);
    assert_eq!(value("SELECT nullIfZero(-3)"), ConstantValue::Integer(-3));
}
#[test]
fn rejects_wrong_arguments() {
    assert!(parse_select("SELECT nullIfZero()").is_err());
    assert!(parse_select("SELECT nullIfZero('x')").is_err());
    assert!(parse_select("SELECT nullIfZero(1, 2)").is_err());
}
