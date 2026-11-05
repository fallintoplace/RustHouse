use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn regexpextract_handles_constant_inputs() {
    for (sql, expected) in [
        ("SELECT regexpExtract('abc-123','([0-9]+)')", ConstantValue::Text("123".into())),
        ("SELECT regexpExtract('abc-123','([a-z]+)-([0-9]+)', 2)", ConstantValue::Text("123".into())),
        ("SELECT regexpExtract('abc','[0-9]+')", ConstantValue::Text("".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT regexpextract(NULL, NULL)"), ConstantValue::Null);
}

#[test]
fn regexpextract_rejects_malformed_inputs() {
    for sql in [
        "SELECT regexpExtract('x', '[')",
        "SELECT regexpExtract('x')",
        "SELECT regexpExtract('x', 'a', -1)",
        "SELECT regexpExtract('x', 'a', 10)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
