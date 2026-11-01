use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn substring_uses_one_based_positions_and_optional_lengths() {
    for (sql, expected) in [
        ("SELECT SUBSTRING('abcdef', 2, 3)", "bcd"),
        ("SELECT substring('abcdef', 4)", "def"),
        ("SELECT SUBSTRING('abc', 9, 1)", ""),
        ("SELECT SUBSTRING('abc', 2, 0)", ""),
        ("SELECT SUBSTRING('abc', 1, 99)", "abc"),
        ("SELECT SUBSTRING('é😀Z', 2, 1)", "😀"),
        ("SELECT SUBSTRING('é😀Z', 3)", "Z"),
        ("SELECT SUBSTRING(UPPER('abc'), 2, 1)", "B"),
        ("SELECT SUBSTRING('ab', LENGTH('ab'), 1)", "b"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT SUBSTRING(NULL, 1)"), ConstantValue::Null);
    assert_eq!(value("SELECT SUBSTRING('ab', NULL)"), ConstantValue::Null);
    assert_eq!(
        value("SELECT SUBSTRING('ab', 1, NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn substring_rejects_invalid_positions_lengths_and_syntax() {
    for sql in [
        "SELECT SUBSTRING('abc')",
        "SELECT SUBSTRING('abc', 0)",
        "SELECT SUBSTRING('abc', -1)",
        "SELECT SUBSTRING('abc', 1, -1)",
        "SELECT SUBSTRING('abc', '1')",
        "SELECT SUBSTRING('abc', 1, 2, 3)",
        "SELECT SUBSTRING('abc', 1, )",
        "SELECT SUBSTRING('abc', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn substring_counts_original_unicode_characters_with_nested_operations() {
    assert_eq!(
        value("SELECT SUBSTRING('é😀xyz', 2, 3)"),
        ConstantValue::Text("😀xy".into())
    );
    assert_eq!(
        value("SELECT SUBSTRING('é😀xyz', 3, 99)"),
        ConstantValue::Text("xyz".into())
    );
    assert_eq!(
        value("SELECT SUBSTRING('é😀xyz', 6)"),
        ConstantValue::Text(String::new())
    );
}
