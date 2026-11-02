use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn trim_left_defaults_to_ascii_whitespace_not_only_spaces() {
    for (query, expected) in [
        ("SELECT trimLeft('\t\r  Alpha  ')", "Alpha  "),
        ("SELECT ltrim('\u{000B}\u{000C}Alpha')", "Alpha"),
        ("SELECT trimLeft('xxAlpha', 'x')", "Alpha"),
        ("SELECT trimLeft('Alpha  ')", "Alpha  "),
    ] {
        assert_eq!(value(query), ConstantValue::Text(expected.into()), "{query}");
    }
    assert_eq!(value("SELECT trimLeft(NULL)"), ConstantValue::Null);
}
