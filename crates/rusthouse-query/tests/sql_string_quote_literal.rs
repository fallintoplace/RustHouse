use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn quotes_unicode_and_embedded_apostrophes() {
    assert_eq!(
        value("SELECT QUOTE_LITERAL('hello')"),
        ConstantValue::Text("'hello'".into())
    );
    assert_eq!(
        value("SELECT QUOTE_LITERAL('O''Reilly')"),
        ConstantValue::Text("'O''Reilly'".into())
    );
    assert_eq!(
        value("SELECT QUOTE_LITERAL('🙂')"),
        ConstantValue::Text("'🙂'".into())
    );
    assert_eq!(
        value("SELECT QUOTE_LITERAL('')"),
        ConstantValue::Text("''".into())
    );
    assert_eq!(value("SELECT QUOTE_LITERAL(NULL)"), ConstantValue::Null);
}

#[test]
fn quote_literal_rejects_invalid_arity() {
    for sql in [
        "SELECT QUOTE_LITERAL()",
        "SELECT QUOTE_LITERAL('a', 'b')",
        "SELECT QUOTE_LITERAL(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
