use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn trim_supports_character_set_and_default_whitespace() {
    for (query, expected) in [
        ("SELECT trim('\tAlpha\r')", "Alpha"),
        ("SELECT trim('xxAlphaxx', 'x')", "Alpha"),
        ("SELECT trim('  Alpha  ', 'x')", "  Alpha  "),
    ] {
        assert_eq!(value(query), ConstantValue::Text(expected.into()), "{query}");
    }
    assert_eq!(value("SELECT trim('abc', NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT trim(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT trim('abc', 'a', 'b')").is_err());
}
