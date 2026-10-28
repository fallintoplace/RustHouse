use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn clickhouse_compatibility_cases() {
    for (sql, expected) in [
        (
            "SELECT port('https://example.com:8123/path')",
            ConstantValue::Integer(8123),
        ),
        (
            "SELECT port('https://example.com')",
            ConstantValue::Integer(0),
        ),
        (
            "SELECT port('http://example.com', 443)",
            ConstantValue::Integer(443),
        ),
        (
            "SELECT port('http://[::1]:8080/x')",
            ConstantValue::Integer(8080),
        ),
        (
            "SELECT port(NULL)",
            ConstantValue::Null,
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT port()",
        "SELECT port('x', -1)",
        "SELECT port('x', 65536)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
