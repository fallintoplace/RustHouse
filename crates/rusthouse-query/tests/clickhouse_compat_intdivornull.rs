use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn intdivornull_handles_zero_overflow_and_nulls() {
    for (sql, expected) in [
        ("SELECT intDivOrNull(7, 2)", ConstantValue::Integer(3)),
        ("SELECT intDivOrNull(-7, 2)", ConstantValue::Integer(-3)),
        ("SELECT intDivOrNull(7, 0)", ConstantValue::Null),
        ("SELECT intDivOrNull(-9223372036854775807 - 1, -1)", ConstantValue::Null),
        ("SELECT intDivOrNull(NULL, 2)", ConstantValue::Null),
        ("SELECT intDivOrNull(1, NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn intdivornull_rejects_bad_arguments() {
    for sql in [
        "SELECT intDivOrNull()",
        "SELECT intDivOrNull(1)",
        "SELECT intDivOrNull(1, 2, 3)",
        "SELECT intDivOrNull('invalid', 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
