use crate::{ConstantValue, Projection, parse_select};

#[test]
fn uses_big_endian_six_octet_mac_format() {
    for (sql, expected) in [
        ("SELECT MACNumToString(1108152157446)", "01:02:03:04:05:06"),
        ("SELECT MACNumToString(0)", "00:00:00:00:00:00"),
        (
            "SELECT MACNumToString(281474976710655)",
            "FF:FF:FF:FF:FF:FF",
        ),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Text(expected.into())),
            "{sql}",
        );
    }
    assert!(parse_select("SELECT MACNumToString(-1)").is_err());
    assert!(parse_select("SELECT MACNumToString(281474976710656)").is_err());
}
