use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn isipv6multicast_handles_ipv6_input_and_null() {
    for (address, expected) in [
        ("ff02::1", 1),
        ("ff05::2", 1),
        ("2001:db8::1", 0),
        ("::1", 0),
    ] {
        let sql = format!("SELECT isIPv6Multicast('{address}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT isIPv6Multicast(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT isIPv6Multicast('not-ipv6')").is_err());
    assert!(parse_select("SELECT isIPv6Multicast('::1', '::2')").is_err());
}
