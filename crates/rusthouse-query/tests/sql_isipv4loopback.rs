use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn isipv4loopback_recognizes_network_boundaries() {
    for (address, expected) in [
        ("127.0.0.1", 1),
        ("127.255.255.254", 1),
        ("8.8.8.8", 0),
        ("0.0.0.0", 0),
    ] {
        let sql = format!("SELECT isIPv4Loopback('{address}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT isIPv4Loopback(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT isIPv4Loopback('invalid')").is_err());
    assert!(parse_select("SELECT isIPv4Loopback('8.8.8.8', '8.8.4.4')").is_err());
}
