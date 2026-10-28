use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn cut_query_preserves_fragment() {
    for (sql, expected) in [
        (
            "SELECT cutQueryString('http://example.com/path?q=abc#top')",
            "http://example.com/path#top",
        ),
        (
            "SELECT cutQueryString('http://example.com/path?q=abc')",
            "http://example.com/path",
        ),
        (
            "SELECT cutQueryString('http://example.com/path#top')",
            "http://example.com/path#top",
        ),
        ("SELECT cutQueryString('abc')", "abc"),
        ("SELECT cutQueryString('?q=1#x')", "#x"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT cutQueryString(NULL)"), ConstantValue::Null);
}
#[test]
fn cut_query_rejects_invalid_calls() {
    for sql in [
        "SELECT cutQueryString()",
        "SELECT cutQueryString(1)",
        "SELECT cutQueryString('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
