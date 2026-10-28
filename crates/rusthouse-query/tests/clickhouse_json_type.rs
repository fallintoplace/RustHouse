use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn json_function_results() {
    assert_eq!(value("SELECT JSONType('{\"a\":[1,2]}')"), ConstantValue::Text("Object".into()));
    assert_eq!(value("SELECT JSONType('{\"a\":[1,2]}','a')"), ConstantValue::Text("Array".into()));
    assert_eq!(value("SELECT JSONType('{\"a\":\"ok\"}','a')"), ConstantValue::Text("String".into()));
    assert_eq!(value("SELECT JSONType('{\"a\":true}','a')"), ConstantValue::Text("Bool".into()));
    assert_eq!(value("SELECT JSONType('{\"a\":true}','missing')"), ConstantValue::Text("Null".into()));
    assert_eq!(value("SELECT JSONType(NULL)"), ConstantValue::Null);
}

#[test]
fn invalid_json_and_arity_fail_closed() {
    for sql in [
        "SELECT JSONType('garbage')",
        "SELECT JSONType('{}', 5)",
        "SELECT JSONType('{}', 'a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
