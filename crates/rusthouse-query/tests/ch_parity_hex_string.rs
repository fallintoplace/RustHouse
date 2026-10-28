use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn clickhouse_constant_function() {
    for (sql, expected) in [
        ("SELECT hex('Hi')", ConstantValue::Text("4869".into())),
        ("SELECT hex('é')", ConstantValue::Text("C3A9".into())),
        ("SELECT hex(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn invalid_arguments() {
    for sql in [
        "SELECT hex()",
        "SELECT hex(1)",
        "SELECT hex('x', 'y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
