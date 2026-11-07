use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn constant_examples() {
    for (sql, expected) in [
        ("SELECT extractDigits('order-123-45')", ConstantValue::Text("12345".into())),
        ("SELECT extractDigits('a1-b2🙂3')", ConstantValue::Text("123".into())),
        ("SELECT extractDigits('nothing')", ConstantValue::Text("".into())),
        ("SELECT extractDigits('')", ConstantValue::Text("".into())),
        ("SELECT extractDigits(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn invalid_arguments_are_rejected() {
    for sql in [
        "SELECT extractDigits()",
        "SELECT extractDigits(9)",

    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
