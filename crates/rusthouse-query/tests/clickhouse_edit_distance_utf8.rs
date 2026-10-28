use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn edit_distance_utf8_evaluates_constant_expressions() {
    for (sql, expected) in [
        ("SELECT editDistanceUTF8('kitten','sitting')", ConstantValue::Integer(3)),
        ("SELECT editDistanceUTF8('我是谁','我是我')", ConstantValue::Integer(1)),
        ("SELECT editDistanceUTF8('','abc')", ConstantValue::Integer(3)),
        ("SELECT editDistanceUTF8(NULL,'x')", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn edit_distance_utf8_rejects_invalid_arguments() {
    for sql in [
        "SELECT editDistanceUTF8('x')",
        "SELECT editDistanceUTF8('x',1)",
        "SELECT editDistanceUTF8('x','y','z')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
