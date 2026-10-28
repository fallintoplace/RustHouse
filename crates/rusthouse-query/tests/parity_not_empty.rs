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
        ("SELECT notEmpty('RustHouse')", ConstantValue::Integer(1)),
        ("SELECT notEmpty('')", ConstantValue::Integer(0)),
        ("SELECT notEmpty(' ')", ConstantValue::Integer(1)),
        ("SELECT notEmpty(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT notEmpty()",
        "SELECT notEmpty('x', 'y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
