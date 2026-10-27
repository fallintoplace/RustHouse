use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn split_part_handles_unicode_missing_fields_and_nested_calls() {
    for (sql, expected) in [
        ("SELECT SPLIT_PART('a,b,c', ',', 2)", "b"),
        ("SELECT split_part('a,b,c', ',', 4)", ""),
        ("SELECT SPLIT_PART('a,,c', ',', 2)", ""),
        ("SELECT SPLIT_PART('é::🙂::Z', '::', 2)", "🙂"),
        ("SELECT SPLIT_PART('single', ',', 1)", "single"),
        ("SELECT SPLIT_PART(LOWER('A-B'), '-', 1)", "a"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    for sql in [
        "SELECT SPLIT_PART(NULL, ',', 1)",
        "SELECT SPLIT_PART('a', NULL, 1)",
        "SELECT SPLIT_PART('a', ',', NULL)",
    ] {
        assert_eq!(value(sql), ConstantValue::Null, "{sql}");
    }
}

#[test]
fn split_part_rejects_invalid_arguments() {
    for sql in [
        "SELECT SPLIT_PART('a', ',', 0)",
        "SELECT SPLIT_PART('a', ',', -1)",
        "SELECT SPLIT_PART('a', '', 1)",
        "SELECT SPLIT_PART('a', ',')",
        "SELECT SPLIT_PART('a', ',', '1')",
        "SELECT SPLIT_PART('a', ',', 1, 2)",
        "SELECT SPLIT_PART('a', ',', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
