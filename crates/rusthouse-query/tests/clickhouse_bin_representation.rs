use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn encodes_integer_bits_in_big_endian_bytes() {
    for (sql, expected) in [
        ("SELECT bin(0)", "00000000"),
        ("SELECT bin(1)", "00000001"),
        ("SELECT bin(255)", "11111111"),
        ("SELECT bin(257)", "0000000100000001"),
        ("SELECT bin(-1)", "1111111111111111111111111111111111111111111111111111111111111111"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT bin(NULL)"), ConstantValue::Null);
}
#[test]
fn validates_types_and_arity() {
    for sql in ["SELECT bin()", "SELECT bin('1')", "SELECT bin(1,2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
