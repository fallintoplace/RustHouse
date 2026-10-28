use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn ipv4_text_decodes_big_endian_uint32() {
    for (sql, expected) in [
        ("SELECT IPv4StringToNum('192.168.0.1')", 3232235521),
        ("SELECT IPv4StringToNum('0.0.0.0')", 0),
        ("SELECT IPv4StringToNum('255.255.255.255')", 4294967295),
        ("SELECT INET_ATON('127.0.0.1')", 2130706433),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT IPv4StringToNum(NULL)"), ConstantValue::Null);
}
#[test]
fn invalid_ipv4_addresses_fail_closed() {
    for sql in [
        "SELECT IPv4StringToNum('999.0.0.1')",
        "SELECT IPv4StringToNum('127.0.0')",
        "SELECT IPv4StringToNum('1.2.3.4/8')",
        "SELECT IPv4StringToNum('::1')",
        "SELECT IPv4StringToNum('1.2.3.4', '5.6.7.8')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
