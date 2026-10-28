use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn formats_integer_and_string_constants() {
    for (sql, expected) in [
        ("SELECT toString(-42)", ConstantValue::Text("-42".into())),
        ("SELECT toString(2 + 3)", ConstantValue::Text("5".into())),
        ("SELECT toString('猫')", ConstantValue::Text("猫".into())),
        ("SELECT toString(concat('a', 'b'))", ConstantValue::Text("ab".into())),
        ("SELECT toString(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_wrong_arity() {
    for sql in ["SELECT toString()", "SELECT toString('a', 'b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
