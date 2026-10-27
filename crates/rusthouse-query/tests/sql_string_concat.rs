use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn concat_joins_strings_and_supports_nested_calls() {
    for (sql, expected) in [
        ("SELECT CONCAT('Hello', ' ', 'SQL')", "Hello SQL"),
        ("SELECT concat('', '')", ""),
        ("SELECT CONCAT(LOWER('HI'), UPPER('there'))", "hiTHERE"),
        ("SELECT CONCAT('a''b', 'c')", "a'bc"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT CONCAT(NULL, 'hello')"), ConstantValue::Null);
    assert_eq!(value("SELECT CONCAT('x', NULL)"), ConstantValue::Null);
    assert_eq!(
        value("SELECT LENGTH(CONCAT('abc', 'd'))"),
        ConstantValue::Integer(4)
    );
}

#[test]
fn concat_rejects_bad_arity_types_and_table_scans() {
    let too_many = format!("SELECT CONCAT({})", vec!["'x'"; 17].join(","));
    for sql in [
        "SELECT CONCAT()",
        "SELECT CONCAT('one')",
        "SELECT CONCAT('x', 10)",
        "SELECT CONCAT('x',)",
        "SELECT CONCAT('x', 'y'",
        "SELECT CONCAT('x', 'y') FROM numbers(1)",
        too_many.as_str(),
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
