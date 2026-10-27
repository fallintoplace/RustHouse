use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn reverse_handles_unicode_empty_text_and_nested_calls() {
    for (sql, expected) in [
        ("SELECT REVERSE('abc')", "cba"),
        ("SELECT reverse('aé🙂')", "🙂éa"),
        ("SELECT REVERSE('')", ""),
        ("SELECT REVERSE(LOWER('AbC'))", "cba"),
        ("SELECT CHAR_LENGTH(REVERSE('aé🙂'))", "3"),
    ] {
        if expected == "3" {
            assert_eq!(value(sql), ConstantValue::Integer(3), "{sql}");
        } else {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
    }
    assert_eq!(value("SELECT REVERSE(NULL)"), ConstantValue::Null);
}

#[test]
fn reverse_rejects_bad_arity_types_and_table_queries() {
    for sql in [
        "SELECT REVERSE()",
        "SELECT REVERSE('a', 'b')",
        "SELECT REVERSE(123)",
        "SELECT REVERSE('x') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
