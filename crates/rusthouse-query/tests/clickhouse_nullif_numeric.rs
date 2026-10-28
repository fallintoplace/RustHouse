use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn handles_numeric_and_string_nullif() {
    for (sql, expected) in [
        ("SELECT nullIf(4,4)", ConstantValue::Null),
        ("SELECT nullIf(4,5)", ConstantValue::Integer(4)),
        ("SELECT nullIf(-2,abs(-2))", ConstantValue::Integer(-2)),
        ("SELECT nullIf('a','a')", ConstantValue::Null),
        ("SELECT nullIf('a','b')", ConstantValue::Text("a".into())),
        ("SELECT nullIf(NULL,1)", ConstantValue::Null),
        ("SELECT nullIf(1,NULL)", ConstantValue::Integer(1)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}
#[test]
fn validates_nullif_arity() {
    for sql in ["SELECT nullIf()", "SELECT nullIf(1)", "SELECT nullIf(1,2,3)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
