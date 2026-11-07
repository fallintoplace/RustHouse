use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_is_uuid() {
    for (sql, expected) in [
        (
            "SELECT isUUID('550e8400-e29b-41d4-a716-446655440000')",
            ConstantValue::Integer(1),
        ),
        (
            "SELECT isUUID('550e8400-e29b-41d4-a716-44665544000g')",
            ConstantValue::Integer(0),
        ),
        ("SELECT isUUID('')", ConstantValue::Integer(0)),
        ("SELECT isUUID(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_is_uuid_arity() {
    for sql in ["SELECT isUUID()", "SELECT isUUID('x', 'y')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
