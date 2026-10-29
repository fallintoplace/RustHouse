use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).expect(sql).projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn ascii_fast_path_and_unicode_fallback() {
    assert_eq!(value("SELECT upperUTF8('hello')"), ConstantValue::Text("HELLO".into()));
    assert_eq!(value("SELECT upperUTF8('café')"), ConstantValue::Text("CAFÉ".into()));
    assert_eq!(value("SELECT upperUTF8('')"), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT upperUTF8(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_calls() {
    assert!(parse_select("SELECT upperUTF8()").is_err());
    assert!(parse_select("SELECT upperUTF8('a', 'b')").is_err());
}
