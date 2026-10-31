use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(number) => ConstantValue::Integer(number),
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn lower_folds_ascii_and_propagates_null() {
    assert_eq!(
        value("SELECT LOWER('HeLLo')"),
        ConstantValue::Text("hello".into())
    );
    assert_eq!(
        value("SELECT lower('A''B')"),
        ConstantValue::Text("a'b".into())
    );
    assert_eq!(
        value("SELECT LOWER('123 !')"),
        ConstantValue::Text("123 !".into())
    );
    assert_eq!(value("SELECT LOWER(NULL)"), ConstantValue::Null);
    assert_eq!(
        value("SELECT LENGTH(LOWER('ABC'))"),
        ConstantValue::Integer(3)
    );
}

#[test]
fn lower_rejects_non_strings_extra_arguments_and_table_scans() {
    for sql in [
        "SELECT LOWER()",
        "SELECT LOWER(9)",
        "SELECT LOWER('a', 'b')",
        "SELECT LOWER(CHAR_LENGTH('a'))",
        "SELECT LOWER('A'",
        "SELECT LOWER('A') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn lower_folds_unicode_and_checks_expanded_result_size() {
    assert_eq!(
        value("SELECT LOWER('MÜNCHEN İ')"),
        ConstantValue::Text("münchen i\u{307}".into())
    );
    assert!(parse_select("SELECT LOWER(REPEAT('İ', 500000))").is_err());
}

#[test]
fn lower_keeps_unicode_case_expansion_with_nested_expressions() {
    assert_eq!(
        value("SELECT LOWER(UPPER('München Straße'))"),
        ConstantValue::Text("münchen strasse".into())
    );
    assert_eq!(
        value("SELECT LOWER('İ')"),
        ConstantValue::Text("i\u{307}".into())
    );
}
