use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn replaceregexpall_replaces_matches() {
    for (sql, expected) in [
        (
            "SELECT replaceRegexpAll('abc123def456', '[0-9]+', 'X')",
            ConstantValue::Text("abcXdefX".into()),
        ),
        ("SELECT replaceRegexpAll('hello', 'z', 'X')", ConstantValue::Text("hello".into())),
        ("SELECT replaceRegexpAll('aaaa', 'a', 'z')", ConstantValue::Text("zzzz".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT replaceregexpall(NULL, 'a', 'b')"), ConstantValue::Null);
    assert_eq!(value("SELECT replaceregexpall('a', NULL, 'b')"), ConstantValue::Null);
    assert_eq!(value("SELECT replaceregexpall('a', 'a', NULL)"), ConstantValue::Null);
}

#[test]
fn replaceregexpall_rejects_invalid_arguments() {
    for sql in [
        "SELECT replaceregexpall('a', '[', 'b')",
        "SELECT replaceregexpall('a', 'a')",
        "SELECT replaceregexpall('a', 'a', 'b', 'c')",
        "SELECT replaceregexpall(1, 'a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
