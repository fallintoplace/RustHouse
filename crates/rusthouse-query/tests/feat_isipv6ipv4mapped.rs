use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn classify_addresses() {
    for (sql, expected) in [
        ("SELECT isIPv6IPv4Mapped('::ffff:192.0.2.1')", 1),
        ("SELECT isIPv6IPv4Mapped('::ffff:c000:201')", 1),
        ("SELECT isIPv6IPv4Mapped('::192.0.2.1')", 0),
        ("SELECT isIPv6IPv4Mapped('2001:db8::1')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPv6IPv4Mapped(NULL)"), ConstantValue::Null);
}

#[test]
fn reject_malformed_addresses_or_extra_arguments() {
    for sql in [
        "SELECT isIPv6IPv4Mapped('invalid')",
        "SELECT isIPv6IPv4Mapped()",
        "SELECT isIPv6IPv4Mapped('x', 'y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
