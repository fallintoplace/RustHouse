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
        ("SELECT STARTS_WITH('alphabet', 'alpha')", 1),
        ("SELECT starts_with('alpha', 'beta')", 0),
        ("SELECT STARTS_WITH('éclair', 'é')", 1),
        ("SELECT STARTS_WITH('', '')", 1),
        ("SELECT STARTS_WITH(REVERSE('abc'), 'c')", 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT STARTS_WITH()",
        "SELECT STARTS_WITH('a')",
        "SELECT STARTS_WITH('a', 1)",
        "SELECT STARTS_WITH('a', 'a', 'a')",
        "SELECT STARTS_WITH('a', 'a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
#[test]
fn null_inputs_propagate() {
    assert_eq!(value("SELECT STARTS_WITH(NULL, 'a')"), ConstantValue::Null);
    assert_eq!(value("SELECT STARTS_WITH('a', NULL)"), ConstantValue::Null);
}
