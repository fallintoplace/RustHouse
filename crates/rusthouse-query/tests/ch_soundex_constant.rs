use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn soundex_encodes_ascii_names() {
    for (query, expected) in [
        ("SELECT soundex('aksel')", ConstantValue::Text("A240".into())),
        ("SELECT soundex('Robert')", ConstantValue::Text("R163".into())),
        ("SELECT soundex('Rupert')", ConstantValue::Text("R163".into())),
        ("SELECT soundex('Ashcraft')", ConstantValue::Text("A261".into())),
        ("SELECT soundex(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(query), expected, "{query}");
    }
    assert!(parse_select("SELECT soundex('abc', 'def')").is_err());
}
