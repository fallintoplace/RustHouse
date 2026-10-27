use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn rpad_pads_truncates_and_counts_unicode_chars() {
    for (sql, expected) in [
        ("SELECT RPAD('cat', 5, '.')", "cat.."),
        ("SELECT rpad('abc', 6, 'xy')", "abcxyx"),
        ("SELECT RPAD('abcdef', 3, '.')", "abc"),
        ("SELECT RPAD('猫', 3, '🙂')", "猫🙂🙂"),
        ("SELECT RPAD('x', 0, '.')", ""),
        ("SELECT RPAD('x', 3)", "x  "),
        ("SELECT RPAD('é', 3)", "é  "),
        ("SELECT RPAD('abc', 3, '')", "abc"),
        ("SELECT RPAD(UPPER('ab'), 4, '-')", "AB--"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    for sql in [
        "SELECT RPAD(NULL, 2, '-')",
        "SELECT RPAD('x', NULL, '-')",
        "SELECT RPAD('x', 2, NULL)",
    ] {
        assert_eq!(value(sql), ConstantValue::Null, "{sql}");
    }
}

#[test]
fn rpad_rejects_invalid_widths_padding_and_oversize_results() {
    for sql in [
        "SELECT RPAD('x', -1, '.')",
        "SELECT RPAD('x', 2, '')",
        "SELECT RPAD('x', 1048577, '.')",
        "SELECT RPAD(1, 2, '.')",
        "SELECT RPAD('x', 2, '.', '.')",
        "SELECT RPAD('x', 2, '.') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
