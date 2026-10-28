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
            "SELECT topLevelDomain('https://www.example.com:8123/p')",
            ConstantValue::Text("com".into()),
        ),
        ("SELECT topLevelDomain('example.co.uk')", ConstantValue::Text("uk".into())),
        ("SELECT topLevelDomain('localhost')", ConstantValue::Text("".into())),
        ("SELECT topLevelDomain(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT topLevelDomain()",
        "SELECT topLevelDomain('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
