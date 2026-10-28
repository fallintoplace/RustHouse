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
            "SELECT cutURLParameter('https://x.test/?a=1&b=2#a', 'a')",
            ConstantValue::Text("https://x.test/?b=2#a".into()),
        ),
        (
            "SELECT cutURLParameter('http://x/?a=1&b=2&a=3', 'a')",
            ConstantValue::Text("http://x/?b=2&a=3".into()),
        ),
        (
            "SELECT cutURLParameter('http://x/?a&b=2', 'a')",
            ConstantValue::Text("http://x/?a&b=2".into()),
        ),
        (
            "SELECT cutURLParameter(NULL, 'a')",
            ConstantValue::Null,
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT cutURLParameter('x')",
        "SELECT cutURLParameter('x', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
