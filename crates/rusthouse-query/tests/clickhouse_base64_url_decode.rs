use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn decodes_url_alphabet_and_optional_padding() {
    for (sql, expected) in [
        ("SELECT base64URLDecode('8J-Mng==')", "🌞"),
        ("SELECT base64URLDecode('Zm9v')", "foo"),
        ("SELECT base64URLDecode('Zg')", "f"),
        ("SELECT base64URLDecode('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT base64URLDecode(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_alphabet_or_length() {
    for sql in ["SELECT base64URLDecode('a')", "SELECT base64URLDecode('%%%%')",
                "SELECT base64URLDecode(1)", "SELECT base64URLDecode('a','b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
