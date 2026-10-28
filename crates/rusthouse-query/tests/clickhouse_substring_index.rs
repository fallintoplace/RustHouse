use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn handles_signed_occurrence_counts() {
    for (sql, expected) in [
        ("SELECT substringIndex('a.b.c', '.', 1)", "a"),
        ("SELECT substringIndex('a.b.c', '.', 2)", "a.b"),
        ("SELECT substringIndex('a.b.c', '.', -1)", "c"),
        ("SELECT substringIndex('a.b.c', '.', -2)", "b.c"),
        ("SELECT substringIndex('a.b.c', '.', 0)", ""),
        ("SELECT substringIndex('a.b.c', '/', 2)", "a.b.c"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT substringIndex(NULL,'.',1)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_arguments() {
    for sql in ["SELECT substringIndex()", "SELECT substringIndex('a','.')",
                "SELECT substringIndex(2,'.',1)", "SELECT substringIndex('a','.',1,2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
