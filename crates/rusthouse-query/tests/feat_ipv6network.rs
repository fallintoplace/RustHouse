use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn computes_network_addresses() {
    for (sql, expected) in [
        ("SELECT IPv6Network('2001:db8:abcd:1234::1', 32)", "2001:db8::"),
        ("SELECT IPv6Network('2001:db8::1', 128)", "2001:db8::1"),
        ("SELECT IPv6Network('2001:db8::1', 0)", "::"),
        ("SELECT IPv6Network('fe80:12::1', 16)", "fe80::"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT IPv6Network(NULL, 24)"), ConstantValue::Null);
    assert_eq!(value("SELECT IPv6Network('::1', NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_addresses_prefixes_and_arity() {
    for sql in [
        "SELECT IPv6Network('bad', 24)",
        "SELECT IPv6Network('::1', -1)",
        "SELECT IPv6Network('::1', 129)",
        "SELECT IPv6Network()",
        "SELECT IPv6Network('::1')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
