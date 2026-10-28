use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn identifies_ipv4_addresses_and_rejects_ipv6() {
    for (sql, expected) in [
        ("SELECT isIPv4String('127.0.0.1')", 1),
        ("SELECT isIPv4String('0.0.0.0')", 1),
        ("SELECT isIPv4String('255.255.255.255')", 1),
        ("SELECT isIPv4String('256.1.2.3')", 0),
        ("SELECT isIPv4String('::ffff:127.0.0.1')", 0),
        ("SELECT isIPv4String('')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPv4String(NULL)"), ConstantValue::Null);
}
#[test]
fn validates_ipv4_function_arity_and_types() {
    for sql in ["SELECT isIPv4String()", "SELECT isIPv4String(123)", "SELECT isIPv4String('x', 'y')", "SELECT isIPv4String('x') FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
