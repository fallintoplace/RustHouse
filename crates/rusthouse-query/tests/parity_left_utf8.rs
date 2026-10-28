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
        ("SELECT leftUTF8('Привет', 4)", ConstantValue::Text("Прив".into())),
        ("SELECT leftUTF8('Привет', -4)", ConstantValue::Text("Пр".into())),
        ("SELECT leftUTF8('ABC', 0)", ConstantValue::Text("".into())),
        ("SELECT leftUTF8(NULL, 1)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}
#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT leftUTF8('x')",
        "SELECT leftUTF8('x', 'a')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
