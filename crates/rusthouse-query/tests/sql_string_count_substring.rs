use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn counts_nonoverlapping_substring_occurrences() {
    assert_eq!(value("SELECT COUNT_SUBSTRING('banana', 'ana')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT COUNT_SUBSTRING('aaaa', 'aa')"), ConstantValue::Integer(2));
    assert_eq!(value("SELECT COUNT_SUBSTRING('🙂🙂x🙂', '🙂')"), ConstantValue::Integer(3));
    assert_eq!(value("SELECT COUNT_SUBSTRING('abc', 'z')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT COUNT_SUBSTRING(NULL, 'x')"), ConstantValue::Null);
}

#[test]
fn count_substring_rejects_empty_needles_and_wrong_arity() {
    for sql in ["SELECT COUNT_SUBSTRING('abc', '')", "SELECT COUNT_SUBSTRING('abc')", "SELECT COUNT_SUBSTRING('abc', 'a', 'b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
