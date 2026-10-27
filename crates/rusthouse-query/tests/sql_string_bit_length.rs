use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn bit_length_handles_unicode_and_nulls() {
    for (sql, expected) in [
        ("SELECT BIT_LENGTH('abc')", 24),
        ("SELECT bit_length('é')", 16),
        ("SELECT BIT_LENGTH('🙂')", 32),
        ("SELECT BIT_LENGTH('')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT BIT_LENGTH(NULL)"), ConstantValue::Null);
}

#[test]
fn bit_length_rejects_invalid_arguments() {
    for sql in [
        "SELECT BIT_LENGTH()",
        "SELECT BIT_LENGTH(1)",
        "SELECT BIT_LENGTH('a','b')",
        "SELECT BIT_LENGTH('a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
