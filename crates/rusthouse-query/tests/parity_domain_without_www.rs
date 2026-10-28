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
            "SELECT domainWithoutWWW('https://www.example.com/p')",
            ConstantValue::Text("example.com".into()),
        ),
        (
            "SELECT domainWithoutWWW('http://www.www.example.com/')",
            ConstantValue::Text("www.example.com".into()),
        ),
        ("SELECT domainWithoutWWW('example.org')", ConstantValue::Text("example.org".into())),
        ("SELECT domainWithoutWWW(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT domainWithoutWWW()",
        "SELECT domainWithoutWWW('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
