use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn formats_ipv4_unsigned_integers() {
    assert_eq!(value("SELECT IPv4NumToStringClassC(3232235521)"), ConstantValue::Text("192.168.0.xxx".into()));
    assert_eq!(value("SELECT IPv4NumToStringClassC(0)"), ConstantValue::Text("0.0.0.xxx".into()));
    assert_eq!(value("SELECT IPv4NumToStringClassC(4294967295)"), ConstantValue::Text("255.255.255.xxx".into()));
    assert_eq!(value("SELECT IPV4NUMTOSTRINGCLASSC(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_out_of_range_or_wrong_typed_input() {
    for sql in [
        "SELECT IPv4NumToStringClassC(-1)",
        "SELECT IPv4NumToStringClassC(4294967296)",
        "SELECT IPv4NumToStringClassC('1.2.3.4')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
