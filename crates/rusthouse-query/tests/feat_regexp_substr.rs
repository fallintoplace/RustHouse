use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn regexp_substr_supports_constant_patterns() {
    for (sql, expected) in [
        (
            "SELECT regexp_substr('abc123','[0-9]+')",
            ConstantValue::Text("123".into()),
        ),
        ("SELECT regexp_substr('abc','[0-9]+')", ConstantValue::Null),
        (
            "SELECT regexp_substr('a-b-c','b')",
            ConstantValue::Text("b".into()),
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(
        value("SELECT regexp_substr(NULL, 'x')"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT regexp_substr('x', NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn regexp_substr_rejects_bad_patterns_and_arguments() {
    for sql in [
        "SELECT regexp_substr('x', '[')",
        "SELECT regexp_substr('x')",
        "SELECT regexp_substr('x', 'x', 'y')",
        "SELECT regexp_substr(1, 'x')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
