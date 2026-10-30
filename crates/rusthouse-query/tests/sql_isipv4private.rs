use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn isipv4private_recognizes_network_boundaries() {
    for (address, expected) in [
        ("10.0.0.1", 1),
        ("172.16.1.1", 1),
        ("192.168.0.1", 1),
        ("8.8.8.8", 0),
        ("169.254.1.1", 0),
    ] {
        let sql = format!("SELECT isIPv4Private('{address}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT isIPv4Private(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT isIPv4Private('invalid')").is_err());
    assert!(parse_select("SELECT isIPv4Private('8.8.8.8', '8.8.4.4')").is_err());
}
