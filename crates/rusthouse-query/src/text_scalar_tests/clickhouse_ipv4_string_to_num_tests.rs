use crate::{ConstantValue, Projection, parse_select};

#[test]
fn converts_ipv4_to_network_order_integer() {
    for (sql, expected) in [
        ("SELECT IPv4StringToNum('0.0.0.0')", 0),
        ("SELECT IPv4StringToNum('192.168.0.1')", 3232235521),
        ("SELECT IPv4StringToNum('255.255.255.255')", 4294967295),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Integer(expected)),
            "{sql}",
        );
    }
    for sql in [
        "SELECT IPv4StringToNum('256.1.2.3')",
        "SELECT IPv4StringToNum('1.2.3')",
        "SELECT IPv4StringToNum('1.2.3.4', 'x')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
