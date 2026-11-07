use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_is_alpha_utf8() {
    for (sql, expected) in [
        ("SELECT isAlphaUTF8('Café')", ConstantValue::Integer(1)),
        ("SELECT isAlphaUTF8('abc2')", ConstantValue::Integer(0)),
        ("SELECT isAlphaUTF8('')", ConstantValue::Integer(0)),
        ("SELECT isAlphaUTF8(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_is_alpha_utf8_bad_arity() {
    for sql in ["SELECT isAlphaUTF8()", "SELECT isAlphaUTF8('a','b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
