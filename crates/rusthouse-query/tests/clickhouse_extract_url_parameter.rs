use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_first_matching_parameter() {
    for (sql, expected) in [
        (
            "SELECT extractURLParameter('http://example.com/?a=1&b=2', 'b')",
            "2",
        ),
        (
            "SELECT extractURLParameter('http://example.com/?a=first&a=second', 'a')",
            "first",
        ),
        (
            "SELECT extractURLParameter('http://example.com/?a=%2F#top', 'a')",
            "%2F",
        ),
        (
            "SELECT extractURLParameter('http://example.com/?a=1#top', 'missing')",
            "",
        ),
        (
            "SELECT extractURLParameter('http://example.com/?a', 'a')",
            "",
        ),
        (
            "SELECT extractURLParameter('http://example.com/#top?a=1', 'a')",
            "",
        ),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT extractURLParameter(NULL, 'a')"),
        ConstantValue::Null
    );
}
#[test]
fn rejects_invalid_parameter_calls() {
    for sql in [
        "SELECT extractURLParameter('a')",
        "SELECT extractURLParameter('a', 1)",
        "SELECT extractURLParameter(1, 'a')",
        "SELECT extractURLParameter('a', 'b', 'c')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
