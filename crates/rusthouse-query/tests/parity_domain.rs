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
            "SELECT domain('https://www.example.com:8123/path')",
            ConstantValue::Text("www.example.com".into()),
        ),
        ("SELECT domain('example.org/x')", ConstantValue::Text("example.org".into())),
        ("SELECT domain('http://u:p@host.net/a')", ConstantValue::Text("host.net".into())),
        ("SELECT domain(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT domain()",
        "SELECT domain('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
