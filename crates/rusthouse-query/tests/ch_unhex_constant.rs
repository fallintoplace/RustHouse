use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn unhex_decodes_even_and_odd_digit_counts() {
    for (query, expected) in [
        ("SELECT unhex('4142')", ConstantValue::Text("AB".into())),
        ("SELECT unhex('041')", ConstantValue::Text("\0A".into())),
        ("SELECT unhex('4')", ConstantValue::Text("\u{0004}".into())),
        ("SELECT unhex('c3a9')", ConstantValue::Text("é".into())),
        ("SELECT unhex('')", ConstantValue::Text(String::new())),
        ("SELECT unhex(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(query), expected, "{query}");
    }
    assert!(parse_select("SELECT unhex('GG')").is_err());
    assert!(parse_select("SELECT unhex('FF')").is_err());
    assert!(parse_select("SELECT unhex('41', '42')").is_err());
}
