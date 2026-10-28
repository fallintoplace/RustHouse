use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn trim_whitespace_preserves_text_semantics() {
    for (sql, expected) in [
        ("SELECT TRIM_WHITESPACE('   hi   ')", "hi"),
        ("SELECT trim_whitespace(' 🙂 ')", "🙂"),
        ("SELECT TRIM_WHITESPACE('  a b  ')", "a b"),
        ("SELECT TRIM_WHITESPACE('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT TRIM_WHITESPACE(NULL)"), ConstantValue::Null);
}

#[test]
fn trim_whitespace_rejects_invalid_arguments() {
    for sql in [
        "SELECT TRIM_WHITESPACE()",
        "SELECT TRIM_WHITESPACE(1)",
        "SELECT TRIM_WHITESPACE('a','b')",
        "SELECT TRIM_WHITESPACE('a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
