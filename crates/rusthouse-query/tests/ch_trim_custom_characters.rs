use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn trim_supports_custom_characters_and_keeps_space_only_default() {
    for (query, expected) in [
        ("SELECT trim(' xxAlphaxx ')", ConstantValue::Text("xxAlphaxx".into())),
        ("SELECT trim('xxAlphaxx', 'x')", ConstantValue::Text("Alpha".into())),
        ("SELECT trim('XXAlphaxx', 'xX')", ConstantValue::Text("Alpha".into())),
        ("SELECT trim('  Alpha  ', 'x')", ConstantValue::Text("  Alpha  ".into())),
        ("SELECT trim('\tAlpha\t')", ConstantValue::Text("\tAlpha\t".into())),
        ("SELECT trim('abc', '')", ConstantValue::Text("abc".into())),
        ("SELECT trim(NULL)", ConstantValue::Null),
        ("SELECT trim('abc', NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(query), expected, "{query}");
    }
    assert!(parse_select("SELECT trim('abc', 'a', 'b')").is_err());
}
