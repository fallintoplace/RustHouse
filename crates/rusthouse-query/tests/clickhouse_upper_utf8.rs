use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn upper_utf8_works() {
    assert_eq!(value("SELECT upperUTF8('привет')"), ConstantValue::Text("ПРИВЕТ".into()));
    assert_eq!(value("SELECT upperUTF8('abc')"), ConstantValue::Text("ABC".into()));
    assert_eq!(value("SELECT upperUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn upper_utf8_rejects_invalid_calls() {
    for sql in [
        "SELECT upperUTF8(4)",
        "SELECT upperUTF8('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
