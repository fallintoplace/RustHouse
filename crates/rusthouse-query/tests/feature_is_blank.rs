use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_is_blank() {
    for (sql, expected) in [
        ("SELECT isBlank('   ')", ConstantValue::Integer(1)),
        ("SELECT isBlank('')", ConstantValue::Integer(1)),
        ("SELECT isBlank(' a ')", ConstantValue::Integer(0)),
        ("SELECT isBlank(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_is_blank_arity() {
    for sql in ["SELECT isBlank()", "SELECT isBlank('x', 'y')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
