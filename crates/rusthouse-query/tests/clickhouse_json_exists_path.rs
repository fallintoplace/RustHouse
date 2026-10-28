use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_clickhouse_cases() {
    assert_eq!(value("SELECT JSON_EXISTS('{\"a\":{\"b\":1}}','$.a.b')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT JSON_EXISTS('{\"a\":[3]}','$.a[0]')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT JSON_EXISTS('{\"a\":[3]}','$.a[1]')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSON_EXISTS('{\"a\":null}','$.a')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT JSON_EXISTS('{\"a\":1}','$')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT JSON_EXISTS(NULL,'$.a')"), ConstantValue::Null);
}

#[test]
fn rejects_unsupported_or_malformed_calls() {
    for sql in [
        "SELECT JSON_EXISTS('invalid','$.a')",
        "SELECT JSON_EXISTS('{}','a')",
        "SELECT JSON_EXISTS('{}','$.a[*]')",
        "SELECT JSON_EXISTS('{}',1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
