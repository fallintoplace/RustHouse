use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn formats_mac_as_uppercase_six_byte_hex() {
    for (sql, expected) in [
        ("SELECT MACNumToString(0)", "00:00:00:00:00:00"),
        ("SELECT MACNumToString(1)", "00:00:00:00:00:01"),
        ("SELECT MACNumToString(149809441867716)", "88:40:3A:91:07:C4"),
        ("SELECT MACNumToString(281474976710655)", "FF:FF:FF:FF:FF:FF"),
        ("SELECT MACNumToString(281474976710656)", "00:00:00:00:00:00"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT MACNumToString(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_wrong_types_and_arity() {
    for sql in ["SELECT MACNumToString()", "SELECT MACNumToString('a')",
                "SELECT MACNumToString(1, 2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
