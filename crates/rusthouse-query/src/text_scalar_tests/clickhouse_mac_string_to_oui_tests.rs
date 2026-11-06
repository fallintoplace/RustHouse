use crate::{ConstantValue, Projection, parse_select};

#[test]
fn extracts_vendor_oui_prefix_of_valid_mac() {
    for (sql, expected) in [
        ("SELECT MACStringToOUI('01:02:03:AA:BB:CC')", 66051),
        ("SELECT MACStringToOUI('ff:ff:ff:ff:ff:ff')", 16777215),
        ("SELECT MACStringToOUI('00:00:00:12:34:56')", 0),
        ("SELECT MACStringToOUI('invalid')", 0),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Integer(expected)),
            "{sql}",
        );
    }
    assert!(parse_select("SELECT MACStringToOUI('x', 'y')").is_err());
}
