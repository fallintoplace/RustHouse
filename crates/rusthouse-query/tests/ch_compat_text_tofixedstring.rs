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
    assert_eq!(value("SELECT toFixedString('ab', 4)"), ConstantValue::Text("ab\0\0".into()));
    assert_eq!(value("SELECT toFixedString('é', 4)"), ConstantValue::Text("é\0\0".into()));
    assert_eq!(value("SELECT toFixedString('ab', 2)"), ConstantValue::Text("ab".into()));
    assert_eq!(value("SELECT toFixedString(NULL, 2)"), ConstantValue::Null);
}
#[test]
fn rejects_wrong_arguments() {
    assert!(parse_select("SELECT toFixedString('a')").is_err());
    assert!(parse_select("SELECT toFixedString('ab', 1)").is_err());
    assert!(parse_select("SELECT toFixedString('a', 0)").is_err());
    assert!(parse_select("SELECT toFixedString('a', 2, 3)").is_err());
}
