use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn decodes_rfc4648_base32_text() {
    for (sql, expected) in [
        ("SELECT base32Decode('JBSWY3DP')", "Hello"),
        ("SELECT base32Decode('MZXW6===')", "foo"),
        ("SELECT base32Decode('MY======')", "f"),
        ("SELECT base32Decode('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT base32Decode(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_bad_padding_and_non_utf8_output() {
    for sql in ["SELECT base32Decode('A')", "SELECT base32Decode('!!!!!!!!')",
                "SELECT base32Decode('MZXW6==A')", "SELECT base32Decode('74======')",
                "SELECT base32Decode(1)", "SELECT base32Decode('A','B')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
