use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn handles_literals_unicode_nulls_and_nested_calls() {
    for (sql, expected) in [
        ("SELECT STRPOS('hello world', 'world')", 7),
        ("SELECT strpos('aé🙂Z', '🙂')", 3),
        ("SELECT positionUTF8('aé🙂Z', '🙂')", 3),
        ("SELECT STRPOS('🙂🙂é', 'é')", 3),
        ("SELECT STRPOS('🙂🙂é', 'missing')", 0),
        ("SELECT STRPOS('🙂🙂é', '')", 1),
        ("SELECT STRPOS('ééa', 'a')", 3),
        ("SELECT STRPOS('abc', 'z')", 0),
        ("SELECT STRPOS('abc', '')", 1),
        ("SELECT STRPOS(REPEAT('ab', 2), 'b')", 2),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT STRPOS()",
        "SELECT STRPOS('abc')",
        "SELECT STRPOS('abc', 2)",
        "SELECT STRPOS('abc', 'a', 'b')",
        "SELECT STRPOS('abc', 'a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
#[test]
fn null_inputs_propagate() {
    assert_eq!(value("SELECT STRPOS(NULL, 'x')"), ConstantValue::Null);
    assert_eq!(value("SELECT STRPOS('x', NULL)"), ConstantValue::Null);
}
