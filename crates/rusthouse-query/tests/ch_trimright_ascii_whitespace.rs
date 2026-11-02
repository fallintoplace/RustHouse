use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn trims_trailing_ascii_whitespace_but_preserves_leading_text() {
    for (query, expected) in [
        ("SELECT trimRight('  Alpha \t\r')", "  Alpha"),
        ("SELECT rtrim('Alpha\u{000B}\u{000C}')", "Alpha"),
        ("SELECT trimRight('Alphaxx', 'x')", "Alpha"),
        ("SELECT trimRight('  Alpha')", "  Alpha"),
    ] {
        assert_eq!(value(query), ConstantValue::Text(expected.into()), "{query}");
    }
    assert_eq!(value("SELECT trimRight(NULL)"), ConstantValue::Null);
}
