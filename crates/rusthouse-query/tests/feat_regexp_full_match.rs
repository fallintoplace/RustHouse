use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn full_match_requires_the_entire_input_to_match() {
    for (sql, expected) in [
        ("SELECT regexp_full_match('ABC123', '[A-Z]+[0-9]+')", 1),
        ("SELECT regexp_full_match('xABC123y', '[A-Z]+[0-9]+')", 0),
        ("SELECT regexp_full_match('', '')", 1),
        ("SELECT regexp_full_match('a\nb', 'a.b')", 0),
        ("SELECT regexp_full_match('a\nb', '(?s)a.b')", 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT regexp_full_match(NULL, 'x')"), ConstantValue::Null);
    assert_eq!(value("SELECT regexp_full_match('x', NULL)"), ConstantValue::Null);
}

#[test]
fn full_match_rejects_invalid_regex_and_arity() {
    for sql in [
        "SELECT regexp_full_match('x', '[')",
        "SELECT regexp_full_match('x')",
        "SELECT regexp_full_match('x', 'x', 'y')",
        "SELECT regexp_full_match(7, 'x')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
