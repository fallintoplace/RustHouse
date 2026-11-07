use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_retain_alnum_utf8() {
    for (sql, expected) in [
        (
            "SELECT retainAlnumUTF8('Café-42_🦀')",
            ConstantValue::Text("Café42".into()),
        ),
        (
            "SELECT retainAlnumUTF8('!*')",
            ConstantValue::Text("".into()),
        ),
        ("SELECT retainAlnumUTF8('')", ConstantValue::Text("".into())),
        ("SELECT retainAlnumUTF8(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_retain_alnum_utf8_bad_arity() {
    for sql in [
        "SELECT retainAlnumUTF8()",
        "SELECT retainAlnumUTF8('a','b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
