use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn computes_network_addresses() {
    for (sql, expected) in [
        ("SELECT IPv4Network('192.0.2.123', 24)", "192.0.2.0"),
        ("SELECT IPv4Network('192.0.2.123', 32)", "192.0.2.123"),
        ("SELECT IPv4Network('192.0.2.123', 0)", "0.0.0.0"),
        ("SELECT IPv4Network('10.12.3.4', 16)", "10.12.0.0"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT IPv4Network(NULL, 24)"), ConstantValue::Null);
    assert_eq!(value("SELECT IPv4Network('1.2.3.4', NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_addresses_prefixes_and_arity() {
    for sql in [
        "SELECT IPv4Network('bad', 24)",
        "SELECT IPv4Network('1.2.3.4', -1)",
        "SELECT IPv4Network('1.2.3.4', 33)",
        "SELECT IPv4Network()",
        "SELECT IPv4Network('1.2.3.4')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
