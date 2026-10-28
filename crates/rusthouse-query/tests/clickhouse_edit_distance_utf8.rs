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
    assert_eq!(value("SELECT editDistanceUTF8('kitten','sitting')"), ConstantValue::Integer(3));
    assert_eq!(value("SELECT editDistanceUTF8('我是谁','我是我')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT editDistanceUTF8('é','e')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT editDistanceUTF8('','привет')"), ConstantValue::Integer(6));
    assert_eq!(value("SELECT editDistanceUTF8('a',NULL)"), ConstantValue::Null);
}
#[test]
fn validate_types_and_arity() {
    assert!(parse_select("SELECT editDistanceUTF8()").is_err());
    assert!(parse_select("SELECT editDistanceUTF8('a')").is_err());
    assert!(parse_select("SELECT editDistanceUTF8(1,'a')").is_err());
    assert!(parse_select("SELECT editDistanceUTF8('a','b','c')").is_err());
}
