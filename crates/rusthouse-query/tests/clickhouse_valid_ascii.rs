use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn clickhouse_parity_examples() {
    assert_eq!(value("SELECT isValidASCII('Hello')"), ConstantValue::Integer(1), "SELECT isValidASCII('Hello')");
    assert_eq!(value("SELECT isValidASCII('')"), ConstantValue::Integer(1), "SELECT isValidASCII('')");
    assert_eq!(value("SELECT isValidASCII('é')"), ConstantValue::Integer(0), "SELECT isValidASCII('é')");
    assert_eq!(value("SELECT isValidASCII('😀')"), ConstantValue::Integer(0), "SELECT isValidASCII('😀')");
    assert_eq!(value("SELECT isvalidascii(NULL)"), ConstantValue::Null);
}
#[test]
fn invalid_arity_and_type_are_rejected() {
    assert!(parse_select("SELECT isValidASCII()").is_err());
    assert!(parse_select("SELECT isValidASCII(3)").is_err());
    assert!(parse_select("SELECT isValidASCII('a','b')").is_err());
}
