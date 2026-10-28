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
        ("SELECT lengthUTF8('naïve')", ConstantValue::Integer(5)),
        ("SELECT lengthUTF8('🌍a')", ConstantValue::Integer(2)),
        ("SELECT lengthUTF8('')", ConstantValue::Integer(0)),
        ("SELECT lengthUTF8(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT lengthUTF8()",
        "SELECT lengthUTF8(123)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
