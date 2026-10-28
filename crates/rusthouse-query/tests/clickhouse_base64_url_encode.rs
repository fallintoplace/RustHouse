use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn uses_url_safe_base64_alphabet() {
    for (sql, expected) in [
        ("SELECT base64URLEncode('foo')", "Zm9v"),
        ("SELECT base64URLEncode('🌞')", "8J-Mng=="),
        ("SELECT base64URLEncode('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT base64URLEncode(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_wrong_arity() {
    for sql in ["SELECT base64URLEncode()", "SELECT base64URLEncode(5)",
                "SELECT base64URLEncode('a','b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
