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
        ("SELECT positionUTF8('ééabc', 'a')", ConstantValue::Integer(3)),
        ("SELECT positionUTF8('ééabc', 'é', 2)", ConstantValue::Integer(2)),
        ("SELECT positionUTF8('ééabc', 'z')", ConstantValue::Integer(0)),
        ("SELECT positionUTF8(NULL, 'a')", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT positionUTF8('abc')",
        "SELECT positionUTF8('abc', 'a', -1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
