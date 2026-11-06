use crate::{ConstantValue, Projection, parse_select};

#[test]
fn mac_oui_reuses_strict_six_byte_parsing() {
    for (sql, expected) in [
        ("SELECT MACStringToOUI('AA:BB:CC:DD:EE:FF')", 0xAA_BB_CC),
        ("SELECT MACStringToOUI('00:00:00:00:00:01')", 0),
        ("SELECT MACStringToOUI('GG:BB:CC:DD:EE:FF')", 0),
        ("SELECT MACStringToOUI('AA:BB:CC')", 0),
    ] {
        assert!(
            matches!(
                parse_select(sql).unwrap().projection,
                Projection::Constants(ref values)
                    if values.len() == 1 && values[0].value == ConstantValue::Integer(expected)
            ),
            "{sql}",
        );
    }
    assert!(parse_select("SELECT MACStringToOUI('AA:BB:CC:DD:EE:FF', 1)").is_err());
}
