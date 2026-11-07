use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn constant_examples() {
    for (sql, expected) in [
        ("SELECT regexp_like_ci('AbC123', 'abc[0-9]+')", ConstantValue::Integer(1)),
        ("SELECT regexp_like_ci('hello', '[0-9]+')", ConstantValue::Integer(0)),
        ("SELECT regexp_like_ci('École', '^é')", ConstantValue::Integer(1)),
        ("SELECT regexp_like_ci(NULL, 'a')", ConstantValue::Null),
        ("SELECT regexp_like_ci('abc', NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn invalid_arguments_are_rejected() {
    for sql in [
        "SELECT regexp_like_ci()",
        "SELECT regexp_like_ci(9)",
        "SELECT regexp_like_ci('x', '[')",
        "SELECT regexp_like_ci('x', 'a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
