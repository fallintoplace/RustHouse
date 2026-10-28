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
    assert_eq!(value("SELECT reverseUTF8('abé')"), ConstantValue::Text("éba".into()), "SELECT reverseUTF8('abé')");
    assert_eq!(value("SELECT reverseUTF8('A😀B')"), ConstantValue::Text("B😀A".into()), "SELECT reverseUTF8('A😀B')");
    assert_eq!(value("SELECT reverseUTF8('')"), ConstantValue::Text("".into()), "SELECT reverseUTF8('')");
    assert_eq!(value("SELECT reverseutf8(NULL)"), ConstantValue::Null);
}
#[test]
fn invalid_arity_and_type_are_rejected() {
    assert!(parse_select("SELECT reverseUTF8()").is_err());
    assert!(parse_select("SELECT reverseUTF8(3)").is_err());
    assert!(parse_select("SELECT reverseUTF8('a','b')").is_err());
}
