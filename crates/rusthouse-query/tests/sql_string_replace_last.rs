use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn replaces_last_match_only() {
    assert_eq!(value("SELECT REPLACE_LAST('banana', 'an', 'X')"), ConstantValue::Text("banXa".into()));
    assert_eq!(value("SELECT REPLACE_LAST('🙂🙂', '🙂', 'x')"), ConstantValue::Text("🙂x".into()));
    assert_eq!(value("SELECT REPLACE_LAST('abc', 'z', 'X')"), ConstantValue::Text("abc".into()));
    assert_eq!(value("SELECT REPLACE_LAST('abc', '', 'X')"), ConstantValue::Text("abc".into()));
    assert_eq!(value("SELECT REPLACE_LAST(NULL, 'a', 'b')"), ConstantValue::Null);
}

#[test]
fn replace_last_rejects_bad_arity() {
    for sql in ["SELECT REPLACE_LAST('a', 'b')", "SELECT REPLACE_LAST('a', 'b', 'c', 'd')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
