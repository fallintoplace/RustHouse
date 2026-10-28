use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn counts_mismatched_bytes() {
    for (sql, expected) in [
        ("SELECT byteHammingDistance('karolin', 'kathrin')", 3),
        ("SELECT byteHammingDistance('abc', 'abc')", 0),
        ("SELECT byteHammingDistance('abc', 'xyz')", 3),
        ("SELECT byteHammingDistance('', '')", 0),
        ("SELECT byteHammingDistance('é', 'ê')", 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT byteHammingDistance(NULL, 'abc')"), ConstantValue::Null);
}
#[test]
fn validates_length_and_types() {
    for sql in ["SELECT byteHammingDistance('a','ab')",
                "SELECT byteHammingDistance(1,'a')", "SELECT byteHammingDistance('a')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
