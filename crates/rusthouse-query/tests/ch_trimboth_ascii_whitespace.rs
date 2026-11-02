use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn btrim_removes_all_ascii_whitespace_by_default() {
    for (query, expected) in [
        ("SELECT trimBoth('\t  Alpha \r')", "Alpha"),
        ("SELECT btrim('\u{000B}Alpha\u{000C}')", "Alpha"),
        ("SELECT trimBoth('xxAlphaxx', 'x')", "Alpha"),
        ("SELECT btrim(' Al pha ')", "Al pha"),
    ] {
        assert_eq!(value(query), ConstantValue::Text(expected.into()), "{query}");
    }
    assert_eq!(value("SELECT trimBoth(NULL)"), ConstantValue::Null);
}
