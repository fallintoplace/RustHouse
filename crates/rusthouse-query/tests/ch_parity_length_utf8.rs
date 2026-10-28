use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn matches_clickhouse_constant_semantics() {
    for (sql, expected) in [
        ("SELECT lengthUTF8('aé猫')", ConstantValue::Integer(3)),
        ("SELECT lengthUtf8('')", ConstantValue::Integer(0)),
        ("SELECT lengthUTF8(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT lengthUTF8()",
        "SELECT lengthUTF8(2)",
        "SELECT lengthUTF8('a','b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
