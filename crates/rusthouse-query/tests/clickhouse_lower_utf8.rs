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
    assert_eq!(value("SELECT lowerUTF8('MÜNCHEN')"), ConstantValue::Text("münchen".into()), "SELECT lowerUTF8('MÜNCHEN')");
    assert_eq!(value("SELECT lowerUTF8('РУСТ')"), ConstantValue::Text("руст".into()), "SELECT lowerUTF8('РУСТ')");
    assert_eq!(value("SELECT lowerUTF8('A')"), ConstantValue::Text("a".into()), "SELECT lowerUTF8('A')");
    assert_eq!(value("SELECT lowerutf8(NULL)"), ConstantValue::Null);
}
#[test]
fn invalid_arity_and_type_are_rejected() {
    assert!(parse_select("SELECT lowerUTF8()").is_err());
    assert!(parse_select("SELECT lowerUTF8(3)").is_err());
    assert!(parse_select("SELECT lowerUTF8('a','b')").is_err());
}
