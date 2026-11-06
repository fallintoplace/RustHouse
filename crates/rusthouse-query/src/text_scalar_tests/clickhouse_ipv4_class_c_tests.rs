use crate::{ConstantValue, Projection, parse_select};

#[test]
fn redacts_low_octet_of_network_order_ipv4() {
    for (sql, expected) in [
        ("SELECT IPv4NumToStringClassC(3232235521)", "192.168.0.xxx"),
        ("SELECT IPv4NumToStringClassC(0)", "0.0.0.xxx"),
        (
            "SELECT IPv4NumToStringClassC(4294967295)",
            "255.255.255.xxx",
        ),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Text(expected.into())),
            "{sql}",
        );
    }
    assert!(parse_select("SELECT IPv4NumToStringClassC(-1)").is_err());
    assert!(parse_select("SELECT IPv4NumToStringClassC(1,2)").is_err());
}
