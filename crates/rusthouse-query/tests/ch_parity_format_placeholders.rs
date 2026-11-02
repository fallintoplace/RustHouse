use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn formats_bounded_automatic_and_indexed_fields() {
    for (sql, expected) in [
        ("SELECT format('{} {}', 'hello', 'world')", "hello world"),
        ("SELECT format('{1} {0} {1}', 'World', 'Hello')", "Hello World Hello"),
        ("SELECT format('{{literal}} {0}', 5)", "{literal} 5"),
        ("SELECT format('{{{0}}}', 'café')", "{café}"),
        ("SELECT format('no fields')", "no fields"),
        ("SELECT format('{}', -12)", "-12"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT format('{}', NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT format(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_braces_indices_and_arity() {
    for sql in [
        "SELECT format('{', 'a')",
        "SELECT format('}', 'a')",
        "SELECT format('{x}', 'a')",
        "SELECT format('{1}', 'a')",
        "SELECT format('{} {1}', 'a', 'b')",
        "SELECT format('{}')",
        "SELECT format()",
        "SELECT format(123)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
