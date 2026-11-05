use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn countmatchescaseinsensitive_handles_regex_constants() {
    for (sql, expected) in [
        ("SELECT countMatchesCaseInsensitive('AbAB','ab')", ConstantValue::Integer(2)),
        ("SELECT countMatchesCaseInsensitive('foo','bar')", ConstantValue::Integer(0)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT countmatchescaseinsensitive(NULL, 'x')"), ConstantValue::Null);
    assert_eq!(value("SELECT countmatchescaseinsensitive('x', NULL)"), ConstantValue::Null);
}

#[test]
fn countmatchescaseinsensitive_rejects_invalid_arguments() {
    for sql in [
        "SELECT countmatchescaseinsensitive('x', '[')",
        "SELECT countmatchescaseinsensitive('x')",
        "SELECT countmatchescaseinsensitive('x', 'x', 'y')",
        "SELECT countmatchescaseinsensitive(1, 'x')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
