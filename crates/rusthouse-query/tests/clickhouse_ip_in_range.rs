use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn matches_ipv4_and_ipv6_cidr_and_rejects_cross_family() {
    for (sql, expected) in [
        ("SELECT isIPAddressInRange('127.0.0.1', '127.0.0.0/8')", 1),
        ("SELECT isIPAddressInRange('192.168.2.1', '192.168.0.0/16')", 1),
        ("SELECT isIPAddressInRange('192.168.2.1', '192.168.1.0/24')", 0),
        ("SELECT isIPAddressInRange('127.0.0.1', '0.0.0.0/0')", 1),
        ("SELECT isIPAddressInRange('127.0.0.1', 'ffff::/16')", 0),
        ("SELECT isIPAddressInRange('::1', '::/0')", 1),
        ("SELECT isIPAddressInRange('2001:db8::1', '2001:db8::/32')", 1),
        ("SELECT isIPAddressInRange('2001:db9::1', '2001:db8::/32')", 0),
        ("SELECT isIPAddressInRange('::1', '127.0.0.0/8')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPAddressInRange(NULL, '10.0.0.0/8')"), ConstantValue::Null);
}
#[test]
fn rejects_malformed_addresses_ranges_and_wrong_argument_counts() {
    for sql in [
        "SELECT isIPAddressInRange()",
        "SELECT isIPAddressInRange('127.0.0.1')",
        "SELECT isIPAddressInRange('127.0.0.1', '127.0.0.0')",
        "SELECT isIPAddressInRange('127.0.0.1', '127.0.0.0/33')",
        "SELECT isIPAddressInRange('::1', '::/129')",
        "SELECT isIPAddressInRange('bad', '127.0.0.0/8')",
        "SELECT isIPAddressInRange('127.0.0.1', '127.0.0.0/bad')",
        "SELECT isIPAddressInRange('127.0.0.1', '127.0.0.0/8', 'extra')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
