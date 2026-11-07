use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn unspecified_addresses_are_identified() {
    for (sql, expected) in [
        ("SELECT isIPv6Unspecified('::')", 1),
        ("SELECT isIPv6Unspecified('0:0:0:0:0:0:0:0')", 1),
        ("SELECT isIPv6Unspecified('::1')", 0),
        ("SELECT isIPv6Unspecified('2001:db8::1')", 0),
        ("SELECT isIPv6Unspecified('ffff::')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPv6Unspecified(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_addresses_and_arity() {
    for sql in [
        "SELECT isIPv6Unspecified('not-an-address')",
        "SELECT isIPv6Unspecified()",
        "SELECT isIPv6Unspecified('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
