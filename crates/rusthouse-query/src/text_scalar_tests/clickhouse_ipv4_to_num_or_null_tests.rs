use crate::{ConstantValue, Projection, parse_select};

#[test]
fn valid_ipv4_is_numeric_and_invalid_input_is_null() {
    for (sql, expected) in [
        ("SELECT IPv4StringToNumOrNull('192.168.0.1')", 3232235521),
        ("SELECT IPv4StringToNumOrNull('0.0.0.0')", 0),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Integer(expected)),
            "{sql}",
        );
    }
    for sql in [
        "SELECT IPv4StringToNumOrNull('invalid')",
        "SELECT IPv4StringToNumOrNull(NULL)",
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1 && values[0].value == ConstantValue::Null),
            "{sql}",
        );
    }
    assert!(parse_select("SELECT IPv4StringToNumOrNull('x', 'y')").is_err());
}
