use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_typed_json_values() {
    assert_eq!(value("SELECT JSONExtractBool('{\"ok\":true}', 'ok')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT JSONExtractBool('{\"ok\":false}', 'ok')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractBool('{\"x\":{\"ok\":true}}', 'x.ok')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT JSONExtractBool('{\"ok\":1}', 'ok')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractBool('{\"ok\":true}', 'missing')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractBool('true')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT JSONExtractBool(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_calls() {
    for sql in [
        "SELECT JSONExtractBool('invalid')",
        "SELECT JSONExtractBool('{}', 1)",
        "SELECT JSONExtractBool('{}', 'x', 'y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
