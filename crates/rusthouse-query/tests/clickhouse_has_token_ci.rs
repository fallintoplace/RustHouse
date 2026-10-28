use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn has_token_is_ascii_case_insensitive() {
    for (sql, expected) in [
        ("SELECT hasTokenCaseInsensitive('Hello, WORLD', 'world')", 1),
        ("SELECT hasTokenCaseInsensitive('one_TWO', 'two')", 0),
        ("SELECT hasTokenCaseInsensitive('one_TWO', 'ONE_two')", 1),
        ("SELECT hasTokenCaseInsensitive('foot', 'FOO')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT hasTokenCaseInsensitive(NULL, 'hi')"),
        ConstantValue::Null
    );
}
#[test]
fn rejects_invalid_case_insensitive_tokens() {
    for sql in [
        "SELECT hasTokenCaseInsensitive('a', 'a b')",
        "SELECT hasTokenCaseInsensitive('a', '')",
        "SELECT hasTokenCaseInsensitive('a')",
        "SELECT hasTokenCaseInsensitive(1, 'a')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
