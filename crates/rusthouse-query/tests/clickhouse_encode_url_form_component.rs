use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn handles_form_encoding_and_unicode() {
    assert_eq!(value("SELECT encodeURLFormComponent('a b+c')"), ConstantValue::Text("a+b%2Bc".into()));
    assert_eq!(value("SELECT encodeURLFormComponent('Grüße!')"), ConstantValue::Text("Gr%C3%BC%C3%9Fe%21".into()));
    assert_eq!(value("SELECT encodeURLFormComponent('a-b_c.~')"), ConstantValue::Text("a-b_c.~".into()));
    assert_eq!(value("SELECT encodeURLFormComponent(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_bad_form_encoding() {
    for sql in [
        "SELECT encodeURLFormComponent()",
        "SELECT encodeURLFormComponent(1)",
        "SELECT encodeURLFormComponent('a','b')",
        "SELECT encodeURLFormComponent('abc') FROM numbers(1)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
