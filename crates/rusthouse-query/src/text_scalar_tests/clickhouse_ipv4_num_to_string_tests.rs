use crate::{ConstantValue, Projection, parse_select};

#[test]
fn formats_network_order_ipv4_integers() {
    for (sql, expected) in [
        ("SELECT IPv4NumToString(0)", "0.0.0.0"),
        ("SELECT IPv4NumToString(3232235521)", "192.168.0.1"),
        ("SELECT IPv4NumToString(4294967295)", "255.255.255.255"),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Text(expected.into())),
            "{sql}",
        );
    }
    for sql in [
        "SELECT IPv4NumToString(-1)",
        "SELECT IPv4NumToString(4294967296)",
        "SELECT IPv4NumToString(1, 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
