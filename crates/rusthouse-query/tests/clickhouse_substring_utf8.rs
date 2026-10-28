use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn substring_utf8_works() {
    assert_eq!(value("SELECT substringUTF8('hé🙂llo', 2, 2)"), ConstantValue::Text("é🙂".into()));
    assert_eq!(value("SELECT substringUTF8('hé🙂llo', 3)"), ConstantValue::Text("🙂llo".into()));
    assert_eq!(value("SELECT substringUTF8(NULL, 1, 1)"), ConstantValue::Null);
}

#[test]
fn substring_utf8_rejects_invalid_calls() {
    for sql in [
        "SELECT substringUTF8('abc', 0)",
        "SELECT substringUTF8('abc', 1, -1)",
        "SELECT substringUTF8('abc')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
