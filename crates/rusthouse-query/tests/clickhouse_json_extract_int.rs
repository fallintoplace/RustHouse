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
    assert_eq!(value("SELECT JSONExtractInt('{\"count\":42}', 'count')"), ConstantValue::Integer(42));
    assert_eq!(value("SELECT JSONExtractInt('{\"a\":{\"n\":-12}}', 'a.n')"), ConstantValue::Integer(-12));
    assert_eq!(value("SELECT JSONExtractInt('{\"x\":2.5}', 'x')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractInt('{\"x\":\"12\"}', 'x')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractInt('{\"x\":9}', 'missing')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractInt('42')"), ConstantValue::Integer(42));
    assert_eq!(value("SELECT JSONExtractInt(NULL, 'x')"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_calls() {
    for sql in [
        "SELECT JSONExtractInt('invalid', 'x')",
        "SELECT JSONExtractInt('{}', 1)",
        "SELECT JSONExtractInt('{}', 'x', 'y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
