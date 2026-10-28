use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(n) => ConstantValue::Integer(n),
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn clickhouse_ipv4_string_to_num_constants() {
    assert_eq!(value("SELECT IPv4StringToNum('192.168.0.1')"), ConstantValue::Integer(3232235521));
    assert_eq!(value("SELECT IPv4StringToNum('127.0.0.1')"), ConstantValue::Integer(2130706433));
    assert_eq!(value("SELECT IPv4StringToNum('0.0.0.0')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT IPv4StringToNum(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT IPv4StringToNum('bad')",
        "SELECT IPv4StringToNum('256.1.2.3')",
        "SELECT IPv4StringToNum()",
        "SELECT IPv4StringToNum('1.1.1.1', '2')",
        "SELECT IPv4StringToNum('1.1.1.1') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
