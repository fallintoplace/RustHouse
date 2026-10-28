use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn replaces_only_first_match_without_losing_unicode() {
    assert_eq!(
        value("SELECT REPLACE_FIRST('banana', 'an', 'X')"),
        ConstantValue::Text("bXana".into())
    );
    assert_eq!(
        value("SELECT REPLACE_FIRST('🙂🙂', '🙂', 'x')"),
        ConstantValue::Text("x🙂".into())
    );
    assert_eq!(
        value("SELECT REPLACE_FIRST('abc', 'z', 'X')"),
        ConstantValue::Text("abc".into())
    );
    assert_eq!(
        value("SELECT REPLACE_FIRST('abc', '', 'X')"),
        ConstantValue::Text("abc".into())
    );
    assert_eq!(
        value("SELECT REPLACE_FIRST(NULL, 'a', 'b')"),
        ConstantValue::Null
    );
}

#[test]
fn replace_first_rejects_bad_arity() {
    for sql in [
        "SELECT REPLACE_FIRST('a', 'b')",
        "SELECT REPLACE_FIRST('a', 'b', 'c', 'd')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
