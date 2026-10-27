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
        ("SELECT ENDS_WITH('alphabet', 'bet')", 1),
        ("SELECT ends_with('alphabet', 'alpha')", 0),
        ("SELECT ENDS_WITH('cake🍰', '🍰')", 1),
        ("SELECT ENDS_WITH('abc', '')", 1),
        ("SELECT ENDS_WITH(UPPER('abc'), 'C')", 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT ENDS_WITH()",
        "SELECT ENDS_WITH('a')",
        "SELECT ENDS_WITH('a', 1)",
        "SELECT ENDS_WITH('a', 'a', 'a')",
        "SELECT ENDS_WITH('a', 'a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
#[test]
fn null_inputs_propagate() {
    assert_eq!(value("SELECT ENDS_WITH(NULL, 'a')"), ConstantValue::Null);
    assert_eq!(value("SELECT ENDS_WITH('a', NULL)"), ConstantValue::Null);
}
