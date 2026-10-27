use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn trim_removes_only_surrounding_spaces() {
    for (sql, expected) in [
        ("SELECT TRIM('  hello world  ')", "hello world"),
        ("SELECT trim('')", ""),
        ("SELECT TRIM('    ')", ""),
        ("SELECT TRIM('x y')", "x y"),
        ("SELECT TRIM(CONCAT(' a', ' '))", "a"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT TRIM(NULL)"), ConstantValue::Null);
    assert_eq!(
        value("SELECT LENGTH(TRIM(' a '))"),
        ConstantValue::Integer(1)
    );
}

#[test]
fn trim_rejects_extra_parameters_and_numbers() {
    for sql in [
        "SELECT TRIM()",
        "SELECT TRIM(1)",
        "SELECT TRIM('a', 'b')",
        "SELECT TRIM('a'",
        "SELECT TRIM(' a ') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
