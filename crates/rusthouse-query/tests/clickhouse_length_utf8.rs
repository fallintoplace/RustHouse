use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        projection => panic!("unexpected projection: {projection:?}"),
    }
}

#[test]
fn length_utf8_works() {
    assert_eq!(value("SELECT lengthUTF8('Aé🙂')"), ConstantValue::Integer(3));
    assert_eq!(value("SELECT lengthUTF8('')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT lengthUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn length_utf8_rejects_invalid_calls() {
    for sql in [
        "SELECT lengthUTF8(3)",
        "SELECT lengthUTF8('abc', 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
