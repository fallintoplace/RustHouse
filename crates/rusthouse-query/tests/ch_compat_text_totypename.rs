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
    assert_eq!(value("SELECT toTypeName(12)"), ConstantValue::Text("Int64".into()));
    assert_eq!(value("SELECT toTypeName('abc')"), ConstantValue::Text("String".into()));
    assert_eq!(value("SELECT toTypeName(NULL)"), ConstantValue::Text("Nullable(Nothing)".into()));
}
#[test]
fn rejects_wrong_arguments() {
    assert!(parse_select("SELECT toTypeName()").is_err());
    assert!(parse_select("SELECT toTypeName(1, 2)").is_err());
}
