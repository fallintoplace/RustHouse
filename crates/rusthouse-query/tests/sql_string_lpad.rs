use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn lpad_pads_truncates_and_counts_unicode_chars() {
    for (sql, expected) in [
        ("SELECT LPAD('cat', 5, '.')", "..cat"),
        ("SELECT lpad('abc', 6, 'xy')", "xyxabc"),
        ("SELECT LPAD('abcdef', 3, '.')", "abc"),
        ("SELECT LPAD('猫', 3, '🙂')", "🙂🙂猫"),
        ("SELECT LPAD('x', 0, '.')", ""),
        ("SELECT LPAD('abc', 3, '')", "abc"),
        ("SELECT LPAD(UPPER('ab'), 4, '-')", "--AB"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    for sql in [
        "SELECT LPAD(NULL, 2, '-')",
        "SELECT LPAD('x', NULL, '-')",
        "SELECT LPAD('x', 2, NULL)",
    ] {
        assert_eq!(value(sql), ConstantValue::Null, "{sql}");
    }
}

#[test]
fn lpad_rejects_invalid_widths_padding_and_oversize_results() {
    for sql in [
        "SELECT LPAD('x', -1, '.')",
        "SELECT LPAD('x', 2, '')",
        "SELECT LPAD('x', 1048577, '.')",
        "SELECT LPAD('x', 2)",
        "SELECT LPAD(1, 2, '.')",
        "SELECT LPAD('x', 2, '.', '.')",
        "SELECT LPAD('x', 2, '.') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
