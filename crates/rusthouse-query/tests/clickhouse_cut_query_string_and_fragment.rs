use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn strips_query_and_fragment() {
    for (sql, expected) in [
        (
            "SELECT cutQueryStringAndFragment('https://example.com/a?q=1#x')",
            "https://example.com/a",
        ),
        (
            "SELECT cutQueryStringAndFragment('https://example.com/a#x')",
            "https://example.com/a",
        ),
        (
            "SELECT cutQueryStringAndFragment('https://example.com/a?q=1')",
            "https://example.com/a",
        ),
        ("SELECT cutQueryStringAndFragment('plain')", "plain"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT cutQueryStringAndFragment(NULL)"),
        ConstantValue::Null
    );
}
#[test]
fn rejects_wrong_argument_types() {
    for sql in [
        "SELECT cutQueryStringAndFragment()",
        "SELECT cutQueryStringAndFragment(1)",
        "SELECT cutQueryStringAndFragment('a','b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
