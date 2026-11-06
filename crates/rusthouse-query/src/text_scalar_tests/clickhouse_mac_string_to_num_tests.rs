use crate::{ConstantValue, Projection, parse_select};

#[test]
fn parses_six_octets_and_defaults_invalid_formats_to_zero() {
    for (sql, expected) in [
        ("SELECT MACStringToNum('01:02:03:04:05:06')", 1108152157446),
        (
            "SELECT MACStringToNum('ff:ff:ff:ff:ff:ff')",
            281474976710655,
        ),
        ("SELECT MACStringToNum('bad')", 0),
        ("SELECT MACStringToNum('01:02:03:04:05')", 0),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Integer(expected)),
            "{sql}",
        );
    }
    assert!(parse_select("SELECT MACStringToNum('x','y')").is_err());
}
