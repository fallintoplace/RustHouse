use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_fragment_without_hash() {
    for (sql, expected) in [
        (
            "SELECT fragment('https://example.com/docs?q=1#chapter')",
            "chapter",
        ),
        ("SELECT fragment('https://example.com/docs#')", ""),
        ("SELECT fragment('https://example.com/docs')", ""),
        ("SELECT fragment('foo#bar#baz')", "bar#baz"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT fragment(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_wrong_fragment_calls() {
    for sql in [
        "SELECT fragment()",
        "SELECT fragment(1)",
        "SELECT fragment('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
