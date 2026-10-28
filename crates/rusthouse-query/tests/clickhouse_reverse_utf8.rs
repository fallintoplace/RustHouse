use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn reverse_utf8_works() {
    assert_eq!(value("SELECT reverseUTF8('Aé🙂')"), ConstantValue::Text("🙂éA".into()));
    assert_eq!(value("SELECT reverseUTF8('')"), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT reverseUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn reverse_utf8_rejects_invalid_calls() {
    for sql in [
        "SELECT reverseUTF8(4)",
        "SELECT reverseUTF8('abc', 'def')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
