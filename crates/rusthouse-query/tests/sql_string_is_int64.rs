use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn is_int64_handles_signed_integer_text() {
    for (sql, expected) in [
        ("SELECT IS_INT64('42')", 1),
        ("SELECT is_int64('-9')", 1),
        ("SELECT IS_INT64('007')", 1),
        ("SELECT IS_INT64('1.2')", 0),
        ("SELECT IS_INT64('9223372036854775808')", 0),
        ("SELECT IS_INT64('')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT IS_INT64(NULL)"), ConstantValue::Null);
}

#[test]
fn is_int64_rejects_bad_arguments() {
    for sql in [
        "SELECT IS_INT64()",
        "SELECT IS_INT64(1)",
        "SELECT IS_INT64('1','2')",
        "SELECT IS_INT64('1') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
