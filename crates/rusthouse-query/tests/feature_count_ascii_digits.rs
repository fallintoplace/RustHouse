use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_count_ascii_digits() {
    for (sql, expected) in [
        ("SELECT countASCIIDigits('room 123-45')", ConstantValue::Integer(5)),
        ("SELECT countASCIIDigits('٣2')", ConstantValue::Integer(1)),
        ("SELECT countASCIIDigits('')", ConstantValue::Integer(0)),
        ("SELECT countASCIIDigits(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_count_ascii_digits_arity() {
    for sql in ["SELECT countASCIIDigits()", "SELECT countASCIIDigits('x','y')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
