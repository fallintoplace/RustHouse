use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn checks_ipv4_conversion() {
    assert_eq!(value("SELECT IPv4StringToNumOrDefault('127.0.0.1')"), ConstantValue::Integer(2130706433));
    assert_eq!(value("SELECT IPv4StringToNumOrDefault('invalid')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT IPv4StringToNumOrDefault('256.0.0.1')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT IPv4StringToNumOrDefault('0.0.0.0')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT IPV4STRINGTONUMORDEFAULT(NULL)"), ConstantValue::Null);
}

#[test]
fn checks_argument_validation() {
    for sql in [
        "SELECT IPv4StringToNumOrDefault(3)",
        "SELECT IPv4StringToNumOrDefault('127.0.0.1', 0)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
