use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_strip_non_ascii() {
    for (sql, expected) in [
        ("SELECT stripNonASCII('café42')", ConstantValue::Text("caf42".into())),
        ("SELECT stripNonASCII('東京')", ConstantValue::Text("".into())),
        ("SELECT stripNonASCII('')", ConstantValue::Text("".into())),
        ("SELECT stripNonASCII(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_strip_non_ascii_arity() {
    for sql in ["SELECT stripNonASCII()", "SELECT stripNonASCII('x','y')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
