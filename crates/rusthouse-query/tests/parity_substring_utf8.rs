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
            "SELECT substringUTF8('Привет', 2, 3)",
            ConstantValue::Text("рив".into()),
        ),
        (
            "SELECT substringUTF8('Привет', -4, 2)",
            ConstantValue::Text("ив".into()),
        ),
        (
            "SELECT substringUTF8('abcd', 3)",
            ConstantValue::Text("cd".into()),
        ),
        (
            "SELECT substringUTF8(NULL, 1)",
            ConstantValue::Null,
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT substringUTF8('abc', 1, -1)",
        "SELECT substringUTF8('abc')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
