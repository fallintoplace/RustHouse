use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn replaceregexpone_replaces_matches() {
    for (sql, expected) in [
        ("SELECT replaceRegexpOne('abc123def456', '[0-9]+', 'X')", ConstantValue::Text("abcXdef456".into())),
        ("SELECT replaceRegexpOne('hello', 'z', 'X')", ConstantValue::Text("hello".into())),
        ("SELECT replaceRegexpOne('aaaa', 'a', 'z')", ConstantValue::Text("zaaa".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT replaceregexpone(NULL, 'a', 'b')"), ConstantValue::Null);
    assert_eq!(value("SELECT replaceregexpone('a', NULL, 'b')"), ConstantValue::Null);
    assert_eq!(value("SELECT replaceregexpone('a', 'a', NULL)"), ConstantValue::Null);
}

#[test]
fn replaceregexpone_rejects_invalid_arguments() {
    for sql in [
        "SELECT replaceregexpone('a', '[', 'b')",
        "SELECT replaceregexpone('a', 'a')",
        "SELECT replaceregexpone('a', 'a', 'b', 'c')",
        "SELECT replaceregexpone(1, 'a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
