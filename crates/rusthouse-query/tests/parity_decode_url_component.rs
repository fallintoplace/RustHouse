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
            "SELECT decodeURLComponent('hello%20world')",
            ConstantValue::Text("hello world".into()),
        ),
        (
            "SELECT decodeURLComponent('%C3%A9')",
            ConstantValue::Text("é".into()),
        ),
        (
            "SELECT decodeURLComponent('a%2Fb')",
            ConstantValue::Text("a/b".into()),
        ),
        (
            "SELECT decodeURLComponent(NULL)",
            ConstantValue::Null,
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT decodeURLComponent()",
        "SELECT decodeURLComponent('%XX')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
