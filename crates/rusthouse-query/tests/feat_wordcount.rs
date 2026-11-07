use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn scalar_examples_and_nulls() {
    for (sql, expected) in [
        ("SELECT wordCount('alpha beta gamma')", ConstantValue::Integer(3)),
        ("SELECT wordCount('  alpha   beta  ')", ConstantValue::Integer(2)),
        ("SELECT wordCount('alpha\\tbeta')", ConstantValue::Integer(2)),
        ("SELECT wordCount('')", ConstantValue::Integer(0)),
        ("SELECT wordCount(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_wrong_arity_and_types() {
    for sql in [
        "SELECT wordCount()",
        "SELECT wordCount('one', 'two')",
        "SELECT wordCount(7)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
