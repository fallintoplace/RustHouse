use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn detects_null_in_constant_expressions() {
    for (sql, expected) in [
        ("SELECT isNull(NULL)", 1),
        ("SELECT isNull('')", 0),
        ("SELECT isNull(42)", 0),
        ("SELECT isNull(abs(-5))", 0),
        ("SELECT isNull(try_to_int64('oops'))", 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
}
#[test]
fn rejects_wrong_arity() {
    for sql in ["SELECT isNull()", "SELECT isNull(NULL, 1)", "SELECT isNull(foo)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
