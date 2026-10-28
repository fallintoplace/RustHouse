use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn removes_fragments_without_touching_queries() {
    for (sql, expected) in [
        (
            "SELECT cutFragment('http://example.com/p?a=1#top')",
            "http://example.com/p?a=1",
        ),
        (
            "SELECT cutFragment('http://example.com/p#')",
            "http://example.com/p",
        ),
        (
            "SELECT cutFragment('http://example.com/p?a=1')",
            "http://example.com/p?a=1",
        ),
        ("SELECT cutFragment('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT cutFragment(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_fragment_arguments() {
    for sql in [
        "SELECT cutFragment()",
        "SELECT cutFragment(1)",
        "SELECT cutFragment('a', 'b')",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
