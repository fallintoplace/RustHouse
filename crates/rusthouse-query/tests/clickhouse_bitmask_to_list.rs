use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn lists_set_bits_as_ascending_powers_of_two() {
    for (sql, expected) in [
        ("SELECT bitmaskToList(0)", ""),
        ("SELECT bitmaskToList(1)", "1"),
        ("SELECT bitmaskToList(10)", "2,8"),
        ("SELECT bitmaskToList(13)", "1,4,8"),
        ("SELECT bitmaskToList(256)", "256"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT bitmaskToList(NULL)"), ConstantValue::Null);
}
#[test]
fn validates_arity() {
    for sql in ["SELECT bitmaskToList()", "SELECT bitmaskToList('a')",
                "SELECT bitmaskToList(2,3)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
