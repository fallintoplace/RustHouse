use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn lowers_utf8_strings_and_preserves_null() {
    for (sql, expected) in [
        ("SELECT lowerUTF8('MÜNCHEN')", "münchen"),
        ("SELECT LOWERUTF8('МОСКВА')", "москва"),
        ("SELECT lowerUTF8('İ')", "i\u{307}"),
        ("SELECT lowerUTF8('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT lowerUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_wrong_types_and_unbounded_output() {
    for sql in ["SELECT lowerUTF8()", "SELECT lowerUTF8(1)", "SELECT lowerUTF8('x','y')", "SELECT lowerUTF8(REPEAT('İ', 500000))", "SELECT lowerUTF8('x') FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
