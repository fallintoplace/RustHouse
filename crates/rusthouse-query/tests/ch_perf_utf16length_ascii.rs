use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).expect(sql).projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn ascii_fast_path_matches_unicode_fallback() {
    assert_eq!(value("SELECT UTF16_LENGTH('hello')"), ConstantValue::Integer(5));
    assert_eq!(value("SELECT UTF16_LENGTH('café')"), ConstantValue::Integer(4));
    assert_eq!(value("SELECT UTF16_LENGTH('🚀')"), ConstantValue::Integer(2));
    assert_eq!(value("SELECT UTF16_LENGTH(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_calls() {
    assert!(parse_select("SELECT UTF16_LENGTH()").is_err());
    assert!(parse_select("SELECT UTF16_LENGTH('a', 'b')").is_err());
}
