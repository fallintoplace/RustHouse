use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn is_empty_handles_constants_and_nulls() {
    for (sql, expected) in [
        ("SELECT IS_EMPTY('')", 1),
        ("SELECT IS_EMPTY(' ')", 0),
        ("SELECT is_empty('🙂')", 0),
        ("SELECT IS_EMPTY(REPLACE('aa','a',''))", 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT IS_EMPTY(NULL)"), ConstantValue::Null);
}

#[test]
fn is_empty_rejects_invalid_arguments() {
    for sql in [
        "SELECT IS_EMPTY()",
        "SELECT IS_EMPTY('a','b')",
        "SELECT IS_EMPTY(0)",
        "SELECT IS_EMPTY('a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
