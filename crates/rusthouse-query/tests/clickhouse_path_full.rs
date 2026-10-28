use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn includes_query_and_fragment_in_url_path() {
    for (sql, expected) in [
        (
            "SELECT pathFull('https://example.com/a/b?q=1#top')",
            "/a/b?q=1#top",
        ),
        ("SELECT pathFull('https://example.com/a')", "/a"),
        ("SELECT pathFull('https://example.com?x=1')", "?x=1"),
        ("SELECT pathFull('https://example.com/')", "/"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT pathFull(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_wrong_path_full_calls() {
    for sql in [
        "SELECT pathFull()",
        "SELECT pathFull(1)",
        "SELECT pathFull('a','b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
