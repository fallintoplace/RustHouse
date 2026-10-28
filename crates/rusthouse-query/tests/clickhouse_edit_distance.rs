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
    assert_eq!(value("SELECT editDistance('kitten','sitting')"), ConstantValue::Integer(3));
    assert_eq!(value("SELECT editDistance('abc','abc')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT editDistance('','abc')"), ConstantValue::Integer(3));
    assert_eq!(value("SELECT editDistance('é','e')"), ConstantValue::Integer(2));
    assert_eq!(value("SELECT editDistance(NULL,'a')"), ConstantValue::Null);
}
#[test]
fn validate_types_and_arity() {
    assert!(parse_select("SELECT editDistance()").is_err());
    assert!(parse_select("SELECT editDistance('a')").is_err());
    assert!(parse_select("SELECT editDistance(1,'a')").is_err());
    assert!(parse_select("SELECT editDistance('a','b','c')").is_err());
}
