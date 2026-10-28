use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn soundex_values() {
    for (sql, expected) in [
        ("SELECT soundex('aksel')", ConstantValue::Text("A240".into())),
        ("SELECT soundex('Robert')", ConstantValue::Text("R163".into())),
        ("SELECT soundex('Rupert')", ConstantValue::Text("R163".into())),
        ("SELECT soundex('Ashcraft')", ConstantValue::Text("A261".into())),
        ("SELECT soundex(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn soundex_invalid_calls() {
    for sql in [
        "SELECT soundex()",
        "SELECT soundex(3)",
        "SELECT soundex('a','b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
