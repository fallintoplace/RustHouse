use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn moduloornull_handles_zero_overflow_and_nulls() {
    for (sql, expected) in [
        ("SELECT moduloOrNull(7, 3)", ConstantValue::Integer(1)),
        ("SELECT moduloOrNull(-7, 3)", ConstantValue::Integer(-1)),
        ("SELECT moduloOrNull(7, 0)", ConstantValue::Null),
        ("SELECT moduloOrNull(-9223372036854775807 - 1, -1)", ConstantValue::Null),
        ("SELECT moduloOrNull(NULL, 2)", ConstantValue::Null),
        ("SELECT moduloOrNull(1, NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn moduloornull_rejects_bad_arguments() {
    for sql in [
        "SELECT moduloOrNull()",
        "SELECT moduloOrNull(1)",
        "SELECT moduloOrNull(1, 2, 3)",
        "SELECT moduloOrNull('invalid', 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
