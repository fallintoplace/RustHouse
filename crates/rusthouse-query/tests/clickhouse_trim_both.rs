use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn clickhouse_trim_both_handles_ascii_whitespace_custom_chars_and_null() {
    for (sql, expected) in [
        ("SELECT trimBoth('  abc  ')", ConstantValue::Text("abc".into())),
        ("SELECT trimBoth('  abc  ', ' ')", ConstantValue::Text("abc".into())),
        ("SELECT trimBoth('$$ClickHouse$$', '$')", ConstantValue::Text("ClickHouse".into())),
        ("SELECT trimBoth('abc', '')", ConstantValue::Text("abc".into())),
        ("SELECT trimBoth(NULL)", ConstantValue::Null),
        ("SELECT trimBoth('abc', NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn clickhouse_trim_both_validates_arity_and_types() {
    for sql in [
        "SELECT trimBoth()",
        "SELECT trimBoth(42)",
        "SELECT trimBoth('abc', 1)",
        "SELECT trimBoth('abc', '', '')",
        "SELECT trimBoth('abc'",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
