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
    assert_eq!(value("SELECT isNotNull(NULL)"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT isNotNull(0)"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT isNotNull('a')"), ConstantValue::Integer(1));
}
#[test]
fn rejects_wrong_arguments() {
    assert!(parse_select("SELECT isNotNull()").is_err());
    assert!(parse_select("SELECT isNotNull(1, 2)").is_err());
}
