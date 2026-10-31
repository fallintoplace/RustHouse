use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn appends_character_only_when_missing() {
    for (sql, expected) in [
        (
            "SELECT appendTrailingCharIfAbsent('https://example.com', '/')",
            "https://example.com/",
        ),
        (
            "SELECT appendTrailingCharIfAbsent('https://example.com/', '/')",
            "https://example.com/",
        ),
        ("SELECT appendTrailingCharIfAbsent('', '/')", ""),
        ("SELECT appendTrailingCharIfAbsent('', '☃')", ""),
        ("SELECT appendTrailingCharIfAbsent('雪', '☃')", "雪☃"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT appendTrailingCharIfAbsent(NULL, '/')"),
        ConstantValue::Null
    );
}
#[test]
fn rejects_non_character_suffixes() {
    for sql in [
        "SELECT appendTrailingCharIfAbsent('a', '')",
        "SELECT appendTrailingCharIfAbsent('a', 'bc')",
        "SELECT appendTrailingCharIfAbsent('a')",
        "SELECT appendTrailingCharIfAbsent(1, '/')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
