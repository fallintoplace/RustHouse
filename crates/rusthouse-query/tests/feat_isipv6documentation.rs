use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn classifies_ipv6_prefixes_and_null() {
    for (sql, expected) in [
        ("SELECT isIPv6Documentation('2001:db8::1')", 1),
        ("SELECT isIPv6Documentation('2001:db8:ffff::1')", 1),
        ("SELECT isIPv6Documentation('2001:db9::1')", 0),
        ("SELECT isIPv6Documentation('fd00::1')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPv6Documentation(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_ipv6_and_wrong_arity() {
    for sql in [
        "SELECT isIPv6Documentation('not-an-address')",
        "SELECT isIPv6Documentation()",
        "SELECT isIPv6Documentation('::1', '::2')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
