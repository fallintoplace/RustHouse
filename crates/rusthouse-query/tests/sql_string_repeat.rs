use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn repeat_builds_bounded_constant_strings_and_nests() {
    for (sql, expected) in [
        ("SELECT REPEAT('ab', 3)", "ababab"),
        ("SELECT repeat('x', 0)", ""),
        ("SELECT REPEAT('', 1000)", ""),
        ("SELECT REPEAT('a''', 2)", "a'a'"),
        ("SELECT LEFT(REPEAT('ha', 3), 4)", "haha"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT REPEAT(NULL, 2)"), ConstantValue::Null);
    assert_eq!(value("SELECT REPEAT('x', NULL)"), ConstantValue::Null);
    assert_eq!(
        value("SELECT LENGTH(REPEAT('abc', 4))"),
        ConstantValue::Integer(12)
    );
}

#[test]
fn repeat_rejects_unbounded_outputs_and_invalid_arguments() {
    for sql in [
        "SELECT REPEAT()",
        "SELECT REPEAT('a')",
        "SELECT REPEAT('a', -1)",
        "SELECT REPEAT('a', 9223372036854775807)",
        "SELECT REPEAT('ab', 600000)",
        "SELECT REPEAT('a', '2')",
        "SELECT REPEAT('a', 1, 2)",
        "SELECT REPEAT('a', 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
