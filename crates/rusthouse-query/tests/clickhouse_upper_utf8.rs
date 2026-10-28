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
    assert_eq!(value("SELECT upperUTF8('München')"), ConstantValue::Text("MÜNCHEN".into()), "SELECT upperUTF8('München')");
    assert_eq!(value("SELECT upperUTF8('привет')"), ConstantValue::Text("ПРИВЕТ".into()), "SELECT upperUTF8('привет')");
    assert_eq!(value("SELECT upperUTF8('mix')"), ConstantValue::Text("MIX".into()), "SELECT upperUTF8('mix')");
    assert_eq!(value("SELECT upperutf8(NULL)"), ConstantValue::Null);
}
#[test]
fn invalid_arity_and_type_are_rejected() {
    assert!(parse_select("SELECT upperUTF8()").is_err());
    assert!(parse_select("SELECT upperUTF8(3)").is_err());
    assert!(parse_select("SELECT upperUTF8('a','b')").is_err());
}
