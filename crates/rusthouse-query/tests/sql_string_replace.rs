use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn replace_handles_nonoverlapping_matches_and_nested_text() {
    for (sql, expected) in [
        ("SELECT REPLACE('banana', 'na', 'NO')", "baNONO"),
        ("SELECT replace('aaaa', 'aa', 'b')", "bb"),
        ("SELECT REPLACE('aba', 'b', '')", "aa"),
        ("SELECT REPLACE('abc', '', 'X')", "abc"),
        ("SELECT REPLACE('abc', 'z', 'X')", "abc"),
        ("SELECT REPLACE(UPPER('abc'), 'B', 'x')", "AxC"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT REPLACE(NULL, 'a', 'b')"), ConstantValue::Null);
    assert_eq!(value("SELECT REPLACE('a', NULL, 'b')"), ConstantValue::Null);
    assert_eq!(value("SELECT REPLACE('a', 'a', NULL)"), ConstantValue::Null);
}

#[test]
fn replace_checks_argument_count_types_and_output_bounds() {
    for sql in [
        "SELECT REPLACE('abc', 'a')",
        "SELECT REPLACE('a', 'a', 'b', 'c')",
        "SELECT REPLACE('a', 'b', 0)",
        "SELECT REPLACE('a', 'b', )",
        "SELECT REPLACE('a', 'b', 'c'",
        "SELECT REPLACE('a', 'b', 'c') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
