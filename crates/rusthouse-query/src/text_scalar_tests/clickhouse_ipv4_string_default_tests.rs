use crate::{ConstantValue, Projection, parse_select};

#[test]
fn invalid_ipv4_strings_return_zero() {
    for (sql, expected) in [
        ("SELECT IPv4StringToNumOrDefault('127.0.0.1')", 2130706433),
        ("SELECT IPv4StringToNumOrDefault('invalid')", 0),
        ("SELECT IPv4StringToNumOrDefault('256.0.0.1')", 0),
        ("SELECT IPv4StringToNumOrDefault('0.0.0.0')", 0),
    ] {
        assert!(
            matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Integer(expected)),
            "{sql}",
        );
    }
    assert!(parse_select("SELECT IPv4StringToNumOrDefault('a', 0)").is_err());
}
