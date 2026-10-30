use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn normalizeipv6_handles_ipv6_input_and_null() {
    for (address, expected) in [
        ("2001:0db8:0000:0000:0000:0000:0000:0001", "2001:db8::1"),
        ("0:0:0:0:0:0:0:1", "::1"),
        ("FD00:0:0:0:0:0:0:1", "fd00::1"),
    ] {
        let sql = format!("SELECT normalizeIPv6('{address}')");
        assert_eq!(scalar(&sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(scalar("SELECT normalizeIPv6(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT normalizeIPv6('not-ipv6')").is_err());
    assert!(parse_select("SELECT normalizeIPv6('::1', '::2')").is_err());
}
