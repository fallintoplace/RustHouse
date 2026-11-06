use crate::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn supported_values_and_invalid_arity() {
    for (sql, expected) in [
        ("SELECT position('éa', 'a')", ConstantValue::Integer(3)),
        ("SELECT position('hello', 'll')", ConstantValue::Integer(3)),
        ("SELECT position('abc', 'Z')", ConstantValue::Integer(0)),
        ("SELECT position(NULL, 'a')", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    for sql in ["SELECT position('abc')", "SELECT position('abc','a','x')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
