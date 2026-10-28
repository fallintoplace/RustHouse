use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(n) => ConstantValue::Integer(n),
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn clickhouse_ipv4_string_to_num_default_constants() {
    assert_eq!(value("SELECT IPv4StringToNumOrDefault('192.168.0.1')"), ConstantValue::Integer(3232235521));
    assert_eq!(value("SELECT IPv4StringToNumOrDefault('bad')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT IPv4StringToNumOrDefault('256.1.2.3')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT IPv4StringToNumOrDefault(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT IPv4StringToNumOrDefault()",
        "SELECT IPv4StringToNumOrDefault('a', 'b')",
        "SELECT IPv4StringToNumOrDefault('a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
