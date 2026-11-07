use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn classifies_network_ranges_and_null() {
    for (sql, expected) in [
        ("SELECT isIPv4Shared('100.64.0.0')", 1),
        ("SELECT isIPv4Shared('100.127.255.255')", 1),
        ("SELECT isIPv4Shared('100.63.255.255')", 0),
        ("SELECT isIPv4Shared('100.128.0.0')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPv4Shared(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_addresses_and_arguments() {
    for sql in [
        "SELECT isIPv4Shared('bad-address')",
        "SELECT isIPv4Shared()",
        "SELECT isIPv4Shared('127.0.0.1', '8.8.8.8')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
