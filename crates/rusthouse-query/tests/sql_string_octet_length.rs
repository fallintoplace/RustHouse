use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn octet_length_handles_unicode_and_nulls() {
    for (sql, expected) in [
        ("SELECT OCTET_LENGTH('é')", 2),
        ("SELECT octet_length('🙂')", 4),
        ("SELECT OCTET_LENGTH('abc')", 3),
        ("SELECT OCTET_LENGTH('')", 0),
        ("SELECT OCTET_LENGTH(CONCAT('a', 'é'))", 3),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT OCTET_LENGTH(NULL)"), ConstantValue::Null);
}

#[test]
fn octet_length_rejects_invalid_arguments() {
    for sql in [
        "SELECT OCTET_LENGTH()",
        "SELECT OCTET_LENGTH('x','y')",
        "SELECT OCTET_LENGTH(42)",
        "SELECT OCTET_LENGTH('x') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
