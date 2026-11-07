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
        ("SELECT isIPv4Unspecified('0.0.0.0')", 1),
        ("SELECT isIPv4Unspecified('0.0.0.1')", 0),
        ("SELECT isIPv4Unspecified('127.0.0.1')", 0),
        ("SELECT isIPv4Unspecified('255.255.255.255')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPv4Unspecified(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_addresses_and_arity() {
    for sql in [
        "SELECT isIPv4Unspecified('not-an-address')",
        "SELECT isIPv4Unspecified()",
        "SELECT isIPv4Unspecified('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
