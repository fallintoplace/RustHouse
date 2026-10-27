use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn contains_is_case_sensitive_and_unicode_aware() {
    assert_eq!(value("SELECT CONTAINS('hello', 'ell')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT CONTAINS('hello', 'Ell')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT CONTAINS('é🙂a', '🙂')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT CONTAINS('abc', '')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT CONTAINS(NULL, 'a')"), ConstantValue::Null);
    assert_eq!(value("SELECT CONTAINS('a', NULL)"), ConstantValue::Null);
}

#[test]
fn contains_rejects_bad_arguments() {
    for sql in ["SELECT CONTAINS('a')", "SELECT CONTAINS('a', 'b', 'c')", "SELECT CONTAINS(1, 'a')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
