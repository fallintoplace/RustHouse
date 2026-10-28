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
        ("SELECT leftPadUTF8('猫', 3, '~')", ConstantValue::Text("~~猫".into())),
        ("SELECT leftPadUTF8('abc', 2)", ConstantValue::Text("ab".into())),
        ("SELECT leftPadUTF8(NULL, 2)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT leftPadUTF8()",
        "SELECT leftPadUTF8('x')",
        "SELECT leftPadUTF8('x', -1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
