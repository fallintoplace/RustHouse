use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_query_without_fragment() {
    for (sql, expected) in [
        (
            "SELECT queryString('https://example.com/p?a=1&b=2#top')",
            "a=1&b=2",
        ),
        ("SELECT queryString('https://example.com/p?')", ""),
        ("SELECT queryString('https://example.com/p#top')", ""),
        (
            "SELECT queryString('https://example.com/p#top?not-query')",
            "",
        ),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT queryString(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_wrong_arity_and_type() {
    for sql in [
        "SELECT queryString()",
        "SELECT queryString(1)",
        "SELECT queryString('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
