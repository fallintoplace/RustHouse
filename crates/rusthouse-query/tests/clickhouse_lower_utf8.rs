use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn lower_utf8_works() {
    assert_eq!(value("SELECT lowerUTF8('ПРИВЕТ')"), ConstantValue::Text("привет".into()));
    assert_eq!(value("SELECT lowerUTF8('ABC')"), ConstantValue::Text("abc".into()));
    assert_eq!(value("SELECT lowerUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn lower_utf8_rejects_invalid_calls() {
    for sql in [
        "SELECT lowerUTF8(4)",
        "SELECT lowerUTF8('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
