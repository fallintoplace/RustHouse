use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn isipv4multicast_recognizes_network_boundaries() {
    for (address, expected) in [
        ("224.0.0.1", 1),
        ("239.255.255.250", 1),
        ("223.255.255.255", 0),
        ("240.0.0.1", 0),
    ] {
        let sql = format!("SELECT isIPv4Multicast('{address}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT isIPv4Multicast(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT isIPv4Multicast('invalid')").is_err());
    assert!(parse_select("SELECT isIPv4Multicast('8.8.8.8', '8.8.4.4')").is_err());
}
