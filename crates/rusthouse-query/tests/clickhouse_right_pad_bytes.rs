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
        ("SELECT rightPad('abc', 5, 'x')", "abcxx"),
        ("SELECT rightPad('abcd', 2)", "ab"),
        ("SELECT rightPad('é', 4, 'x')", "éxx"),
        ("SELECT rightPad('a', 3)", "a  "),
        ("SELECT rightPad('', 0)", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT rightPad(NULL, 3)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_sizes_and_broken_utf8() {
    for sql in ["SELECT rightPad('x', -1)",
                "SELECT rightPad('é', 1)", "SELECT rightPad('x', 3, '')",
                "SELECT rightPad('x')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
