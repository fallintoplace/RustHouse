use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_url_path_without_query() {
    for (sql, expected) in [
        (
            "SELECT path('https://clickhouse.com/docs/faq?q=1#top')",
            "/docs/faq",
        ),
        ("SELECT path('http://localhost:8123/')", "/"),
        ("SELECT path('https://example.com?x=1')", ""),
        ("SELECT path('https://example.com/a/b')", "/a/b"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT path(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_path_calls() {
    for sql in ["SELECT path()", "SELECT path(1)", "SELECT path('a','b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
