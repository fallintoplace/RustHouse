use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn right_reads_bounded_suffixes_of_constant_strings() {
    for (sql, expected) in [
        ("SELECT RIGHT('abcdef', 3)", "def"),
        ("SELECT right('abc', 0)", ""),
        ("SELECT RIGHT('abc', 9)", "abc"),
        ("SELECT RIGHT('', 2)", ""),
        ("SELECT RIGHT('é😀Z', 2)", "😀Z"),
        ("SELECT RIGHT(TRIM(' a '), 1)", "a"),
        ("SELECT RIGHT('abc', LENGTH('ab'))", "bc"),
        ("SELECT RIGHT(CONCAT('ab', 'cd'), 2)", "cd"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT RIGHT(NULL, 1)"), ConstantValue::Null);
    assert_eq!(value("SELECT RIGHT('a', NULL)"), ConstantValue::Null);
}

#[test]
fn right_rejects_negative_counts_bad_arity_and_from_clause() {
    for sql in [
        "SELECT RIGHT()",
        "SELECT RIGHT('abc')",
        "SELECT RIGHT('abc', -1)",
        "SELECT RIGHT('abc', '1')",
        "SELECT RIGHT('abc', 1, 2)",
        "SELECT RIGHT('abc', 1",
        "SELECT RIGHT('abc', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn right_counts_unicode_codepoints_without_cutting_utf8_bytes() {
    assert_eq!(
        value("SELECT RIGHT('é😀xyz', 4)"),
        ConstantValue::Text("😀xyz".into())
    );
    assert_eq!(
        value("SELECT RIGHT('é😀xyz', 1)"),
        ConstantValue::Text("z".into())
    );
}
