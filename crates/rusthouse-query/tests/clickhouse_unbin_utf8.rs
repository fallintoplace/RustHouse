use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn decodes_complete_bytes_into_utf8_text() {
    for (sql, expected) in [
        ("SELECT unbin('01100001')", "a"),
        ("SELECT unbin('0100000101000010')", "AB"),
        ("SELECT unbin('1100001110101001')", "é"),
        ("SELECT unbin('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT unbin(NULL)"), ConstantValue::Null);
}
#[test]
fn refuses_broken_bit_groups_and_utf8() {
    for sql in ["SELECT unbin('101')", "SELECT unbin('0011000X')",
                "SELECT unbin('11111111')", "SELECT unbin()", "SELECT unbin(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
