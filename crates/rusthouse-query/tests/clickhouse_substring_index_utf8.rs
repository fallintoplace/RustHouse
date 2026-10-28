use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn clickhouse_examples_and_boundaries() {
    assert_eq!(value("SELECT substringIndexUTF8('a.b.c','.',2)"), ConstantValue::Text("a.b".into()));
    assert_eq!(value("SELECT substringIndexUTF8('a.b.c','.',-1)"), ConstantValue::Text("c".into()));
    assert_eq!(value("SELECT substringIndexUTF8('a.b.c','.',0)"), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT substringIndexUTF8('日.月.火','.',2)"), ConstantValue::Text("日.月".into()));
    assert_eq!(value("SELECT substringIndexUTF8(NULL,'.',1)"), ConstantValue::Null);
}
#[test]
fn validate_types_and_arity() {
    assert!(parse_select("SELECT substringIndexUTF8('a')").is_err());
    assert!(parse_select("SELECT substringIndexUTF8('a','.')").is_err());
    assert!(parse_select("SELECT substringIndexUTF8('a','.',1,2)").is_err());
    assert!(parse_select("SELECT substringIndexUTF8(1,'.',2)").is_err());
}
