use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn returns_fallback_for_invalid_integer_text() {
    for (sql, expected) in [
        ("SELECT toInt64OrDefault('23')", ConstantValue::Integer(23)),
        ("SELECT toInt64OrDefault('bad')", ConstantValue::Integer(0)),
        ("SELECT toInt64OrDefault('bad', 17)", ConstantValue::Integer(17)),
        ("SELECT toInt64OrDefault('-7', 17)", ConstantValue::Integer(-7)),
        ("SELECT toInt64OrDefault('9223372036854775808', 9)", ConstantValue::Integer(9)),
        ("SELECT toInt64OrDefault(NULL, 42)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}
#[test]
fn validates_arity_and_default_type() {
    for sql in ["SELECT toInt64OrDefault()", "SELECT toInt64OrDefault(5)",
                "SELECT toInt64OrDefault('a','x')", "SELECT toInt64OrDefault('a',1,2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
