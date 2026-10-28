use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn detects_non_null_constant_expressions() {
    for (sql, expected) in [
        ("SELECT isNotNull(NULL)", 0),
        ("SELECT isNotNull('')", 1),
        ("SELECT isNotNull(42)", 1),
        ("SELECT isNotNull(abs(-5))", 1),
        ("SELECT isNotNull(try_to_int64('oops'))", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
}
#[test]
fn rejects_wrong_arity() {
    for sql in ["SELECT isNotNull()", "SELECT isNotNull(NULL, 1)", "SELECT isNotNull(foo)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
