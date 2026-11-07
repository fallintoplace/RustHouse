use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn classify_addresses() {
    for (sql, expected) in [
        ("SELECT isIPv4Benchmark('198.18.0.1')", 1),
        ("SELECT isIPv4Benchmark('198.19.255.255')", 1),
        ("SELECT isIPv4Benchmark('198.17.255.255')", 0),
        ("SELECT isIPv4Benchmark('198.20.0.0')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT isIPv4Benchmark(NULL)"), ConstantValue::Null);
}

#[test]
fn reject_malformed_addresses_or_extra_arguments() {
    for sql in [
        "SELECT isIPv4Benchmark('invalid')",
        "SELECT isIPv4Benchmark()",
        "SELECT isIPv4Benchmark('x', 'y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
