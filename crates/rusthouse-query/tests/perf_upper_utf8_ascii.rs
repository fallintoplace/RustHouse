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
        ("SELECT upperUTF8('abc-123')", ConstantValue::Text("ABC-123".into())),
        ("SELECT upperUTF8('München')", ConstantValue::Text("MÜNCHEN".into())),
        ("SELECT upperUTF8('')", ConstantValue::Text("".into())),
        ("SELECT upperUTF8(upperUTF8('xYz'))", ConstantValue::Text("XYZ".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT upperUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_utf8_function_arguments() {
    for sql in [
        "SELECT upperUTF8()",
        "SELECT upperUTF8('a', 'b')",
        "SELECT upperUTF8(123)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
