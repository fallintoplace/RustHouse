use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn is_numeric_classifies_finite_decimal_text() {
    for (sql, expected) in [
        ("SELECT is_numeric('42')", 1),
        ("SELECT is_numeric('-12.5')", 1),
        ("SELECT is_numeric('1e3')", 1),
        ("SELECT is_numeric('')", 0),
        ("SELECT is_numeric('NaN')", 0),
        ("SELECT is_numeric('inf')", 0),
        ("SELECT is_numeric('1.2.3')", 0),
        ("SELECT is_numeric(' 42 ')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT is_numeric(NULL)"), ConstantValue::Null);
}

#[test]
fn is_numeric_checks_argument_types_and_count() {
    for sql in [
        "SELECT is_numeric()",
        "SELECT is_numeric(1)",
        "SELECT is_numeric('1', '2')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
