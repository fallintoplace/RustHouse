use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn identifies_ipv6_addresses_and_rejects_ipv4() {
    for (sql, expected) in [
        ("SELECT isIPv6String('::')", 1),
        ("SELECT isIPv6String('2001:db8::1')", 1),
        ("SELECT isIPv6String('::ffff:127.0.0.1')", 1),
        ("SELECT isIPv6String('127.0.0.1')", 0),
        ("SELECT isIPv6String('12345::')", 0),
        ("SELECT isIPv6String('')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPv6String(NULL)"), ConstantValue::Null);
}
#[test]
fn validates_ipv6_function_arity_and_types() {
    for sql in [
        "SELECT isIPv6String()",
        "SELECT isIPv6String(123)",
        "SELECT isIPv6String('::', 'x')",
        "SELECT isIPv6String('::') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
