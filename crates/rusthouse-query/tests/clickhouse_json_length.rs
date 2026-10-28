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
    assert_eq!(value("SELECT JSONLength('{\"a\":[1,2],\"b\":true}')"), ConstantValue::Integer(2));
    assert_eq!(value("SELECT JSONLength('{\"a\":[1,2,3]}','a')"), ConstantValue::Integer(3));
    assert_eq!(value("SELECT JSONLength('{\"a\":{\"x\":1,\"y\":2}}','a')"), ConstantValue::Integer(2));
    assert_eq!(value("SELECT JSONLength('{\"a\":\"ok\"}','a')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONLength('{\"a\":true}','missing')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONLength(NULL)"), ConstantValue::Null);
}

#[test]
fn invalid_json_and_arity_fail_closed() {
    for sql in [
        "SELECT JSONLength('garbage')",
        "SELECT JSONLength('{}', 5)",
        "SELECT JSONLength('{}', 'a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
