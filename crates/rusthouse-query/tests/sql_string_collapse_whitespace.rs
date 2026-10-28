use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn collapse_whitespace_preserves_text_semantics() {
    for (sql, expected) in [
        ("SELECT COLLAPSE_WHITESPACE('  one   two  ')", "one two"),
        ("SELECT collapse_whitespace('a b')", "a b"),
        ("SELECT COLLAPSE_WHITESPACE('')", ""),
        ("SELECT COLLAPSE_WHITESPACE('   ')", ""),
        ("SELECT COLLAPSE_WHITESPACE('x   y z')", "x y z"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT COLLAPSE_WHITESPACE(NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn collapse_whitespace_rejects_invalid_arguments() {
    for sql in [
        "SELECT COLLAPSE_WHITESPACE()",
        "SELECT COLLAPSE_WHITESPACE(1)",
        "SELECT COLLAPSE_WHITESPACE('x','y')",
        "SELECT COLLAPSE_WHITESPACE('x') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
