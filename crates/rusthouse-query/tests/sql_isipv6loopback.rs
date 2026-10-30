use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn isipv6loopback_recognizes_network_boundaries() {
    for (address, expected) in [
        ("::1", 1),
        ("::", 0),
        ("2001:db8::1", 0),
        ("fd00::1", 0),
    ] {
        let sql = format!("SELECT isIPv6Loopback('{address}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT isIPv6Loopback(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT isIPv6Loopback('invalid')").is_err());
    assert!(parse_select("SELECT isIPv6Loopback('::1', '::2')").is_err());
}
