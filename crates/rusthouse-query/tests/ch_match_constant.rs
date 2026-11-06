use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn match_supports_bounded_regex_constants() {
    for (sql, expected) in [
        ("SELECT match('hello123', '[0-9]+')", 1),
        ("SELECT match('hello', '[0-9]+')", 0),
        ("SELECT match('', '^$')", 1),
        ("SELECT MATCH('abc', '[A-Z]+')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT match(NULL, 'a')"), ConstantValue::Null);
    assert_eq!(value("SELECT match('a', NULL)"), ConstantValue::Null);
}

#[test]
fn match_rejects_invalid_regex_and_arity() {
    for sql in [
        "SELECT match('a', '[')",
        "SELECT match('a')",
        "SELECT match('a', 'a', 'b')",
        "SELECT match(1, 'a')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
