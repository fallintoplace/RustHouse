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
            "SELECT encodeURLComponent('hello world')",
            ConstantValue::Text("hello%20world".into()),
        ),
        (
            "SELECT encodeURLComponent('é')",
            ConstantValue::Text("%C3%A9".into()),
        ),
        (
            "SELECT encodeURLComponent('a/b?c=d')",
            ConstantValue::Text("a%2Fb%3Fc%3Dd".into()),
        ),
        (
            "SELECT encodeURLComponent(NULL)",
            ConstantValue::Null,
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT encodeURLComponent()",
        "SELECT encodeURLComponent('x','y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
