use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn classifies_valid_and_invalid_inputs() {
    for (sql, expected) in [
        ("SELECT isValidCIDR('192.0.2.0/24')", 1),
        ("SELECT isValidCIDR('2001:db8::/32')", 1),
        ("SELECT isValidCIDR('0.0.0.0/0')", 1),
        ("SELECT isValidCIDR('192.0.2.0/33')", 0),
        ("SELECT isValidCIDR('2001:db8::/129')", 0),
        ("SELECT isValidCIDR('192.0.2.0')", 0),
        ("SELECT isValidCIDR('nonsense/10')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isValidCIDR(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_wrong_arity() {
    for sql in [
        "SELECT isValidCIDR()",
        "SELECT isValidCIDR('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
