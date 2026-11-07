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
        ("SELECT isValidMAC('AA:BB:CC:DD:EE:FF')", 1),
        ("SELECT isValidMAC('00:11:22:33:44:55')", 1),
        ("SELECT isValidMAC('AA:BB:CC:DD:EE')", 0),
        ("SELECT isValidMAC('garbage')", 0),
        ("SELECT isValidMAC('')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isValidMAC(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_wrong_arity() {
    for sql in [
        "SELECT isValidMAC()",
        "SELECT isValidMAC('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
