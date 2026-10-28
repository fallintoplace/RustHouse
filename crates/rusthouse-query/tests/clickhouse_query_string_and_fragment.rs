use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn includes_fragment_after_query() {
    for (sql, expected) in [
        ("SELECT queryStringAndFragment('https://example.com/p?a=1#top')", "a=1#top"),
        ("SELECT queryStringAndFragment('https://example.com/p?a=1')", "a=1"),
        ("SELECT queryStringAndFragment('https://example.com/p#top')", ""),
        ("SELECT queryStringAndFragment('https://example.com/p?')", ""),
    ] { assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}"); }
    assert_eq!(value("SELECT queryStringAndFragment(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_wrong_arity_and_type() {
    for sql in ["SELECT queryStringAndFragment()", "SELECT queryStringAndFragment(1)", "SELECT queryStringAndFragment('a', 'b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
