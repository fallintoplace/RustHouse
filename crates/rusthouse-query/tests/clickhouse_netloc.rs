use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_network_authority() {
    for (sql, expected) in [
        ("SELECT netloc('http://paul@www.example.com:80/')", "paul@www.example.com:80"),
        ("SELECT netloc('https://example.com/a?q=1#x')", "example.com"),
        ("SELECT netloc('https://example.com:8443?q=1')", "example.com:8443"),
        ("SELECT netloc('no-url')", ""),
        ("SELECT netloc('https://')", ""),
    ] { assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}"); }
    assert_eq!(value("SELECT netloc(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_netloc_calls() {
    for sql in ["SELECT netloc()", "SELECT netloc(1)", "SELECT netloc('a', 'b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
