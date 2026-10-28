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
    assert_eq!(value("SELECT trimBoth('  abc  ')"), ConstantValue::Text("abc".into()));
    assert_eq!(value("SELECT trimBoth('$$value$$','$')"), ConstantValue::Text("value".into()));
    assert_eq!(value("SELECT trimBoth('abXYba','ab')"), ConstantValue::Text("XY".into()));
    assert_eq!(value("SELECT trimBoth('xx','')"), ConstantValue::Text("xx".into()));
    assert_eq!(value("SELECT trimBoth(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_bad_arity_and_types() {
    assert!(parse_select("SELECT trimBoth(2)").is_err());
    assert!(parse_select("SELECT trimBoth('a',1)").is_err());
    assert!(parse_select("SELECT trimBoth('a','b','c')").is_err());
}
