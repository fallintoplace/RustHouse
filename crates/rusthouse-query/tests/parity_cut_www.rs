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
            "SELECT cutWWW('https://www.example.com/a?x=1#f')",
            ConstantValue::Text("https://example.com/a?x=1#f".into()),
        ),
        (
            "SELECT cutWWW('www.www.example.com/')",
            ConstantValue::Text("www.example.com/".into()),
        ),
        (
            "SELECT cutWWW('https://example.com')",
            ConstantValue::Text("https://example.com".into()),
        ),
        (
            "SELECT cutWWW(NULL)",
            ConstantValue::Null,
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT cutWWW()",
        "SELECT cutWWW('x', 'y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
