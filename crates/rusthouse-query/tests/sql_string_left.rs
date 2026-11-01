use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn left_reads_bounded_prefixes_of_constant_strings() {
    for (sql, expected) in [
        ("SELECT LEFT('abcdef', 3)", "abc"),
        ("SELECT left('abc', 0)", ""),
        ("SELECT LEFT('abc', 9)", "abc"),
        ("SELECT LEFT('', 2)", ""),
        ("SELECT LEFT('é😀Z', 2)", "é😀"),
        ("SELECT LEFT(TRIM(' a '), 1)", "a"),
        ("SELECT LEFT('abc', LENGTH('ab'))", "ab"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT LEFT(NULL, 1)"), ConstantValue::Null);
    assert_eq!(value("SELECT LEFT('a', NULL)"), ConstantValue::Null);
}

#[test]
fn left_rejects_negative_counts_bad_arity_and_from_clause() {
    for sql in [
        "SELECT LEFT()",
        "SELECT LEFT('abc')",
        "SELECT LEFT('abc', -1)",
        "SELECT LEFT('abc', '1')",
        "SELECT LEFT('abc', 1, 2)",
        "SELECT LEFT('abc', 1",
        "SELECT LEFT('abc', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn left_counts_unicode_codepoints_without_cutting_utf8_bytes() {
    assert_eq!(
        value("SELECT LEFT('é😀xyz', 2)"),
        ConstantValue::Text("é😀".into())
    );
    assert_eq!(
        value("SELECT LEFT('é😀xyz', 3)"),
        ConstantValue::Text("é😀x".into())
    );
}
