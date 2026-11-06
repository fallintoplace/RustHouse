use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn regexp_like_supports_constant_patterns() {
    for (sql, expected) in [
        (
            "SELECT regexp_like('ABC123', '[0-9]+')",
            ConstantValue::Integer(1),
        ),
        (
            "SELECT regexp_like('hello', '[0-9]+')",
            ConstantValue::Integer(0),
        ),
        ("SELECT regexp_like('abc', '^a')", ConstantValue::Integer(1)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT regexp_like(NULL, 'x')"), ConstantValue::Null);
    assert_eq!(value("SELECT regexp_like('x', NULL)"), ConstantValue::Null);
}

#[test]
fn regexp_like_rejects_bad_patterns_and_arguments() {
    for sql in [
        "SELECT regexp_like('x', '[')",
        "SELECT regexp_like('x')",
        "SELECT regexp_like('x', 'x', 'y')",
        "SELECT regexp_like(1, 'x')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
