use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn is_ascii_handles_constants_and_nulls() {
    for (sql, expected) in [
        ("SELECT IS_ASCII('abc 123')", 1),
        ("SELECT is_ascii('')", 1),
        ("SELECT IS_ASCII('é')", 0),
        ("SELECT IS_ASCII('🙂')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT IS_ASCII(NULL)"), ConstantValue::Null);
}

#[test]
fn is_ascii_rejects_invalid_arguments() {
    for sql in [
        "SELECT IS_ASCII()",
        "SELECT IS_ASCII('a','b')",
        "SELECT IS_ASCII(1)",
        "SELECT IS_ASCII('a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
