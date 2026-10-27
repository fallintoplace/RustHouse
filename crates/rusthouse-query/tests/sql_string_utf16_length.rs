use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn utf16_length_handles_constants_and_nulls() {
    for (sql, expected) in [
        ("SELECT UTF16_LENGTH('é')", 1),
        ("SELECT UTF16_LENGTH('🙂')", 2),
        ("SELECT UTF16_LENGTH('a🙂')", 3),
        ("SELECT UTF16_LENGTH('')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT UTF16_LENGTH(NULL)"), ConstantValue::Null);
}

#[test]
fn utf16_length_rejects_invalid_arguments() {
    for sql in [
        "SELECT UTF16_LENGTH()",
        "SELECT UTF16_LENGTH('a','b')",
        "SELECT UTF16_LENGTH(1)",
        "SELECT UTF16_LENGTH('a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
