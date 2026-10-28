use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn verifies_clickhouse_behavior() {
    assert_eq!(value("SELECT trimLeft(' abc ')"), ConstantValue::Text("abc ".into()));
    assert_eq!(value("SELECT trimLeft('ClickHouse','Click')"), ConstantValue::Text("House".into()));
    assert_eq!(value("SELECT trimLeft('abba','ab')"), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT trimLeft(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_bad_arity_and_types() {
    assert!(parse_select("SELECT trimLeft(2)").is_err());
    assert!(parse_select("SELECT trimLeft('a',1)").is_err());
    assert!(parse_select("SELECT trimLeft('a','b','c')").is_err());
}
