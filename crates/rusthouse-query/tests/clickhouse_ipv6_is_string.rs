use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(n) => ConstantValue::Integer(n),
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn clickhouse_ipv6_is_string_constants() {
    assert_eq!(value("SELECT isIPv6String('2001:db8::1')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT isIPv6String('::1')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT isIPv6String('192.168.0.1')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT isIPv6String('bad-ip')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT isIPv6String(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT isIPv6String()",
        "SELECT isIPv6String('::1', 1)",
        "SELECT isIPv6String('::1') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
