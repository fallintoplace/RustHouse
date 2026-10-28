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
            "SELECT positionCaseInsensitiveUTF8('École', 'éc')",
            ConstantValue::Integer(1),
        ),
        (
            "SELECT positionCaseInsensitiveUTF8('éÉabc', 'É', 2)",
            ConstantValue::Integer(2),
        ),
        (
            "SELECT positionCaseInsensitiveUTF8('abc', 'Z')",
            ConstantValue::Integer(0),
        ),
        (
            "SELECT positionCaseInsensitiveUTF8(NULL, 'x')",
            ConstantValue::Null,
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT positionCaseInsensitiveUTF8('x')",
        "SELECT positionCaseInsensitiveUTF8('a', 'x', -1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
