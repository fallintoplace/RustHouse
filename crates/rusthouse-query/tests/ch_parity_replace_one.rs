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
        ("SELECT replaceOne('banana', 'na', '!')", ConstantValue::Text("ba!na".into())),
        ("SELECT replaceOne('xxx', 'y', 'z')", ConstantValue::Text("xxx".into())),
        ("SELECT replaceOne(NULL, 'a', 'b')", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn invalid_arguments() {
    for sql in [
        "SELECT replaceOne()",
        "SELECT replaceOne('a', 'b')",
        "SELECT replaceOne(1, 'b', 'c')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
