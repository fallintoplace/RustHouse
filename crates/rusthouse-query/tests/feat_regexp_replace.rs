use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn regexp_replace_replaces_matches() {
    for (sql, expected) in [
        (
            "SELECT regexp_replace('abab', 'ab', 'X')",
            ConstantValue::Text("Xab".into()),
        ),
        (
            "SELECT regexp_replace('hello', 'z', 'X')",
            ConstantValue::Text("hello".into()),
        ),
        (
            "SELECT regexp_replace('a1b2', '[0-9]', 'X')",
            ConstantValue::Text("aXb2".into()),
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(
        value("SELECT regexp_replace(NULL, 'a', 'b')"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT regexp_replace('a', NULL, 'b')"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT regexp_replace('a', 'a', NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn regexp_replace_rejects_invalid_arguments() {
    for sql in [
        "SELECT regexp_replace('a', '[', 'b')",
        "SELECT regexp_replace('a', 'a')",
        "SELECT regexp_replace('a', 'a', 'b', 'c')",
        "SELECT regexp_replace(1, 'a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
