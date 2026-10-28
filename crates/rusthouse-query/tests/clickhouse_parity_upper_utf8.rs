use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn uppercases_unicode_and_propagates_null() {
    for (sql, expected) in [
        ("SELECT upperUTF8('münchen')", "MÜNCHEN"),
        ("SELECT UPPERUTF8('москва')", "МОСКВА"),
        ("SELECT upperUTF8('')", ""),
        ("SELECT upperUTF8(LOWER('É'))", "É"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT upperUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn validates_arity_and_string_types() {
    for sql in ["SELECT upperUTF8()", "SELECT upperUTF8(1)", "SELECT upperUTF8('x','y')", "SELECT upperUTF8('x') FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
