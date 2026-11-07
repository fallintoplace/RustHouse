use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_is_hex() {
    for (sql, expected) in [
        ("SELECT isHex('0AfF')", ConstantValue::Integer(1)),
        ("SELECT isHex('abc')", ConstantValue::Integer(0)),
        ("SELECT isHex('')", ConstantValue::Integer(0)),
        ("SELECT isHex('xz')", ConstantValue::Integer(0)),
        ("SELECT isHex(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_is_hex_arity() {
    for sql in ["SELECT isHex()", "SELECT isHex('x', 'y')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
