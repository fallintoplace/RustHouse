use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn isipv6uniquelocal_recognizes_network_boundaries() {
    for (address, expected) in [
        ("fd00::1", 1),
        ("fc00::1", 1),
        ("fd12:3456::abcd", 1),
        ("2001:db8::1", 0),
        ("fe80::1", 0),
    ] {
        let sql = format!("SELECT isIPv6UniqueLocal('{address}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT isIPv6UniqueLocal(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT isIPv6UniqueLocal('invalid')").is_err());
    assert!(parse_select("SELECT isIPv6UniqueLocal('::1', '::2')").is_err());
}
