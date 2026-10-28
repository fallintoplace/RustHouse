use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn cidr_membership_matches_ipv4_and_ipv6() {
    for (sql, expected) in [
        ("SELECT isIPAddressInRange('127.0.0.1', '127.0.0.0/8')", 1),
        ("SELECT isIPAddressInRange('192.168.2.1', '192.168.1.0/24')", 0),
        ("SELECT isIPAddressInRange('255.255.255.255', '0.0.0.0/0')", 1),
        ("SELECT isIPAddressInRange('127.0.0.1', 'ffff::/16')", 0),
        ("SELECT isIPAddressInRange('2001:db8::1', '2001:db8::/32')", 1),
        ("SELECT isIPAddressInRange('2001:db9::1', '2001:db8::/32')", 0),
        ("SELECT isIPAddressInRange('::1', '::1/128')", 1),
        ("SELECT isIPAddressInRange('::2', '::1/128')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPAddressInRange(NULL, '0.0.0.0/0')"), ConstantValue::Null);
}
#[test]
fn rejects_malformed_ip_and_cidr_inputs() {
    for sql in [
        "SELECT isIPAddressInRange('invalid', '0.0.0.0/0')",
        "SELECT isIPAddressInRange('1.2.3.4', '1.2.3.0/33')",
        "SELECT isIPAddressInRange('::1', '::/129')",
        "SELECT isIPAddressInRange('1.2.3.4', '1.2.3.0')",
        "SELECT isIPAddressInRange('1.2.3.4', '1.2.3.0/8', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
