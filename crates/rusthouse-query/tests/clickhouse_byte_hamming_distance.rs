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
    assert_eq!(value("SELECT byteHammingDistance('karolin','kathrin')"), ConstantValue::Integer(3));
    assert_eq!(value("SELECT byteHammingDistance('abc','abc')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT byteHammingDistance('ab','abc')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT byteHammingDistance('','x')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT byteHammingDistance('a', NULL)"), ConstantValue::Null);
}
#[test]
fn validate_types_and_arity() {
    assert!(parse_select("SELECT byteHammingDistance()").is_err());
    assert!(parse_select("SELECT byteHammingDistance('abc')").is_err());
    assert!(parse_select("SELECT byteHammingDistance(1,'abc')").is_err());
    assert!(parse_select("SELECT byteHammingDistance('abc','def','ghi')").is_err());
}
