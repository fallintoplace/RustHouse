use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn falls_back_only_for_null_values() {
    for (sql, expected) in [
        ("SELECT ifNull(NULL, 7)", ConstantValue::Integer(7)),
        ("SELECT ifNull(0, 7)", ConstantValue::Integer(0)),
        ("SELECT ifNull(NULL, 'fallback')", ConstantValue::Text("fallback".into())),
        ("SELECT ifNull('', 'fallback')", ConstantValue::Text("".into())),
        ("SELECT ifNull(NULL, NULL)", ConstantValue::Null),
        ("SELECT ifNull(try_to_int64('NaN'), 42)", ConstantValue::Integer(42)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}
#[test]
fn checks_arity() {
    for sql in ["SELECT ifNull()", "SELECT ifNull(1)", "SELECT ifNull(NULL,1,2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
