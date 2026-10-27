use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn contains_evaluates_constant_strings() {
    assert_eq!(value("SELECT CONTAINS('hello', 'ell')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT contains('hello', 'xyz')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT CONTAINS('été', 'té')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT CONTAINS('x', '')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT CONTAINS(NULL, 'a')"), ConstantValue::Null);
    assert_eq!(value("SELECT CONTAINS('a', NULL)"), ConstantValue::Null);
}

#[test]
fn contains_rejects_bad_arguments_and_table_scans() {
    for sql in [
        "SELECT CONTAINS()",
        "SELECT CONTAINS('a')",
        "SELECT CONTAINS('a','b','c')",
        "SELECT CONTAINS('a',1)",
        "SELECT CONTAINS('a','a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
