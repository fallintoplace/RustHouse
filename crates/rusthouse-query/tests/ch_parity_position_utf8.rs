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
        ("SELECT positionUTF8('Motörhead', 'r')", ConstantValue::Integer(5)),
        ("SELECT positionUTF8('éabc', 'missing')", ConstantValue::Integer(0)),
        ("SELECT positionUTF8(NULL, 'a')", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn invalid_arguments() {
    for sql in [
        "SELECT positionUTF8()",
        "SELECT positionUTF8(1, 'a')",
        "SELECT positionUTF8('a')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
