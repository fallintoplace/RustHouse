use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn ascii_returns_unicode_code_points() {
    assert_eq!(value("SELECT ASCII('A')"), ConstantValue::Integer(65));
    assert_eq!(value("SELECT ascii('é')"), ConstantValue::Integer(233));
    assert_eq!(value("SELECT ASCII('🙂!')"), ConstantValue::Integer(128578));
    assert_eq!(value("SELECT ASCII('')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT ASCII(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT ASCII(UPPER('a'))"), ConstantValue::Integer(65));
}

#[test]
fn ascii_rejects_invalid_calls() {
    for sql in ["SELECT ASCII()", "SELECT ASCII(1)", "SELECT ASCII('a', 'b')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
