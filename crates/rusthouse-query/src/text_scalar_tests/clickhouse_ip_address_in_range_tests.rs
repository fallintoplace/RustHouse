use crate::{ConstantValue, Projection, parse_select};

#[test]
fn matches_ipv4_and_ipv6_with_prefix_boundaries() {
    for (sql, expected) in [
        (
            "SELECT isIPAddressInRange('192.168.1.10', '192.168.0.0/16')",
            1,
        ),
        (
            "SELECT isIPAddressInRange('192.167.1.10', '192.168.0.0/16')",
            0,
        ),
        ("SELECT isIPAddressInRange('1.2.3.4', '0.0.0.0/0')", 1),
        (
            "SELECT isIPAddressInRange('2001:db8::1', '2001:db8::/32')",
            1,
        ),
        (
            "SELECT isIPAddressInRange('2001:db9::1', '2001:db8::/32')",
            0,
        ),
        ("SELECT isIPAddressInRange('::1', '::1/128')", 1),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Integer(expected)),
            "{sql}",
        );
    }
    for sql in [
        "SELECT isIPAddressInRange('1.2.3.4', '1.2.3.4/33')",
        "SELECT isIPAddressInRange('1.2.3.4', '::1/128')",
        "SELECT isIPAddressInRange('x', '1.2.3.4/32')",
        "SELECT isIPAddressInRange('1.2.3.4')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
