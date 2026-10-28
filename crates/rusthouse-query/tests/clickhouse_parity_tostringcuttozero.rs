use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn truncates_at_first_nul_byte() {
    assert_eq!(value("SELECT toStringCutToZero('hello')"), ConstantValue::Text("hello".into()));
    assert_eq!(value("SELECT toStringCutToZero('')"), ConstantValue::Text("".into()));
    let sql = format!("SELECT toStringCutToZero('abc{}suffix')", '\0');
    assert_eq!(value(&sql), ConstantValue::Text("abc".into()));
    let leading = format!("SELECT toStringCutToZero('{}abc')", '\0');
    assert_eq!(value(&leading), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT toStringCutToZero(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_wrong_arity_and_types() {
    for sql in ["SELECT toStringCutToZero()", "SELECT toStringCutToZero(1)", "SELECT toStringCutToZero('a','b')", "SELECT toStringCutToZero('a') FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
