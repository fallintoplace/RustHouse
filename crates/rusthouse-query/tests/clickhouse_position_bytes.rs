use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn byte_offsets_and_optional_start() {
    for (sql, expected) in [
        ("SELECT position('banana', 'an')", 2),
        ("SELECT position('banana', 'an', 3)", 4),
        ("SELECT position('Motörhead', 'r')", 6),
        ("SELECT position('abc', 'z')", 0),
        ("SELECT position('abc', '')", 1),
        ("SELECT position('abc', 'a', 50)", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT position(NULL, 'a')"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_arguments() {
    for sql in ["SELECT position()", "SELECT position('a')",
                "SELECT position('abc', 'b', -1)", "SELECT position('a', 'b', 1, 2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
