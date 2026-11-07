use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_retain_digits() {
    for (sql, expected) in [
        (
            "SELECT retainDigits('ID: 00-42!')",
            ConstantValue::Text("0042".into()),
        ),
        (
            "SELECT retainDigits('none')",
            ConstantValue::Text("".into()),
        ),
        ("SELECT retainDigits('')", ConstantValue::Text("".into())),
        ("SELECT retainDigits(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_retain_digits_bad_arity() {
    for sql in ["SELECT retainDigits()", "SELECT retainDigits('a','b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
