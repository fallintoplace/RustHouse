use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn classifies_ipv6_prefixes_and_null() {
    for (sql, expected) in [
        ("SELECT isIPv6LinkLocal('fe80::1')", 1),
        ("SELECT isIPv6LinkLocal('febf::1')", 1),
        ("SELECT isIPv6LinkLocal('fec0::1')", 0),
        ("SELECT isIPv6LinkLocal('fd00::1')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPv6LinkLocal(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_ipv6_and_wrong_arity() {
    for sql in [
        "SELECT isIPv6LinkLocal('not-an-address')",
        "SELECT isIPv6LinkLocal()",
        "SELECT isIPv6LinkLocal('::1', '::2')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
