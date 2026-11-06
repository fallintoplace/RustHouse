use crate::{ConstantValue, Projection, parse_select};

#[test]
fn crc32_matches_ieee_802_3_and_handles_null() {
    for (sql, expected) in [
        ("SELECT CRC32('')", 0),
        ("SELECT CRC32('123456789')", 3_421_780_262),
        ("SELECT CRC32('ClickHouse')", 1_538_217_360),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1 && values[0].value == ConstantValue::Integer(expected)),
            "{sql}",
        );
    }
    assert!(matches!(
        parse_select("SELECT CRC32(NULL)").unwrap().projection,
        Projection::Constants(values)
            if values.len() == 1 && values[0].value == ConstantValue::Null
    ));
    assert!(parse_select("SELECT CRC32('a', 'b')").is_err());
}
