use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn ascii_fast_path_matches_unicode_and_null_behavior() {
    for (sql, expected) in [
        ("SELECT lowerUTF8('ABC-123')", ConstantValue::Text("abc-123".into())),
        ("SELECT lowerUTF8('MünChen')", ConstantValue::Text("münchen".into())),
        ("SELECT lowerUTF8('')", ConstantValue::Text("".into())),
        ("SELECT lowerUTF8(lowerUTF8('XYZ'))", ConstantValue::Text("xyz".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT lowerUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_utf8_function_arguments() {
    for sql in [
        "SELECT lowerUTF8()",
        "SELECT lowerUTF8('a', 'b')",
        "SELECT lowerUTF8(123)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
