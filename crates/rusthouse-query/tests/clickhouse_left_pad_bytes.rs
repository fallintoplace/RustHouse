use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn counts_bytes_not_unicode_codepoints() {
    for (sql, expected) in [
        ("SELECT leftPad('abc', 5, 'x')", "xxabc"),
        ("SELECT leftPad('abcd', 2)", "ab"),
        ("SELECT leftPad('é', 4, 'x')", "xxé"),
        ("SELECT leftPad('a', 3)", "  a"),
        ("SELECT leftPad('', 0)", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT leftPad(NULL, 3)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_sizes_and_broken_utf8() {
    for sql in ["SELECT leftPad('x', -1)",
                "SELECT leftPad('é', 1)", "SELECT leftPad('x', 3, '')",
                "SELECT leftPad('x')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
