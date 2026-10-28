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
        ("SELECT reverseUTF8('é猫a')", ConstantValue::Text("a猫é".into())),
        ("SELECT reverseUTF8('')", ConstantValue::Text("".into())),
        ("SELECT reverseUTF8(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT reverseUTF8()",
        "SELECT reverseUTF8(1)",
        "SELECT reverseUTF8('x','y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
