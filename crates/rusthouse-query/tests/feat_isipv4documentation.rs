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
        ("SELECT isIPv4Documentation('192.0.2.1')", 1),
        ("SELECT isIPv4Documentation('198.51.100.99')", 1),
        ("SELECT isIPv4Documentation('203.0.113.254')", 1),
        ("SELECT isIPv4Documentation('192.0.3.1')", 0),
        ("SELECT isIPv4Documentation('8.8.8.8')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPv4Documentation(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_addresses_and_arguments() {
    for sql in [
        "SELECT isIPv4Documentation('bad-address')",
        "SELECT isIPv4Documentation()",
        "SELECT isIPv4Documentation('127.0.0.1', '8.8.8.8')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
