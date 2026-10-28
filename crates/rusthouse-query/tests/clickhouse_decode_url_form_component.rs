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
    assert_eq!(value("SELECT decodeURLFormComponent('a+b%2Bc')"), ConstantValue::Text("a b+c".into()));
    assert_eq!(value("SELECT decodeURLFormComponent('Gr%C3%BC%C3%9Fe%21')"), ConstantValue::Text("Grüße!".into()));
    assert_eq!(value("SELECT decodeURLFormComponent('%2B+')"), ConstantValue::Text("+ ".into()));
    assert_eq!(value("SELECT decodeURLFormComponent(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_bad_form_encoding() {
    for sql in [
        "SELECT decodeURLFormComponent()",
        "SELECT decodeURLFormComponent(1)",
        "SELECT decodeURLFormComponent('%GG')",
        "SELECT decodeURLFormComponent('%A')",
        "SELECT decodeURLFormComponent('abc') FROM numbers(1)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
