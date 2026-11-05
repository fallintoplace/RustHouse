use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn countmatches_handles_regex_constants() {
    for (sql, expected) in [
        (
            "SELECT countMatches('a1b2c3','[0-9]')",
            ConstantValue::Integer(3),
        ),
        (
            "SELECT countMatches('abc','[0-9]')",
            ConstantValue::Integer(0),
        ),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT countmatches(NULL, 'x')"), ConstantValue::Null);
    assert_eq!(value("SELECT countmatches('x', NULL)"), ConstantValue::Null);
}

#[test]
fn countmatches_rejects_invalid_arguments() {
    for sql in [
        "SELECT countmatches('x', '[')",
        "SELECT countmatches('x')",
        "SELECT countmatches('x', 'x', 'y')",
        "SELECT countmatches(1, 'x')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
