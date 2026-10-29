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
    assert_eq!(value("SELECT isNull(NULL)"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT isNull(0)"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT isNull('')"), ConstantValue::Integer(0));
}
#[test]
fn rejects_wrong_arguments() {
    assert!(parse_select("SELECT isNull()").is_err());
    assert!(parse_select("SELECT isNull(1, 2)").is_err());
}
