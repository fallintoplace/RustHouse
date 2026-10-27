use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_int64_handles_signed_integer_text() {
    for (sql, expected) in [
        ("SELECT TO_INT64('42')", 42),
        ("SELECT to_int64('-7')", -7),
        ("SELECT TO_INT64('9223372036854775807')", i64::MAX),
        ("SELECT TO_INT64('-9223372036854775808')", i64::MIN),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT TO_INT64(NULL)"), ConstantValue::Null);
}

#[test]
fn to_int64_rejects_bad_arguments() {
    for sql in [
        "SELECT TO_INT64()",
        "SELECT TO_INT64('abc')",
        "SELECT TO_INT64('9223372036854775808')",
        "SELECT TO_INT64('1.2')",
        "SELECT TO_INT64(1)",
        "SELECT TO_INT64('1','2')",
        "SELECT TO_INT64('1') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
