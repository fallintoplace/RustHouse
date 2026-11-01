use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(number) => ConstantValue::Integer(number),
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn upper_folds_ascii_and_supports_nested_string_functions() {
    for (sql, expected) in [
        (
            "SELECT UPPER('Hi there')",
            ConstantValue::Text("HI THERE".into()),
        ),
        ("SELECT upper('a''b')", ConstantValue::Text("A'B".into())),
        ("SELECT UPPER(NULL)", ConstantValue::Null),
        (
            "SELECT UPPER(LOWER('MiXeD'))",
            ConstantValue::Text("MIXED".into()),
        ),
        ("SELECT LENGTH(UPPER('ab'))", ConstantValue::Integer(2)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn upper_rejects_bad_arguments_and_from_clause() {
    for sql in [
        "SELECT UPPER()",
        "SELECT UPPER(9)",
        "SELECT UPPER('a', 'b')",
        "SELECT UPPER(LENGTH('a'))",
        "SELECT UPPER('a'",
        "SELECT UPPER('a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn upper_folds_unicode_with_multichar_expansions() {
    assert_eq!(
        value("SELECT UPPER('straße élève')"),
        ConstantValue::Text("STRASSE ÉLÈVE".into())
    );
}

#[test]
fn upper_expands_german_sharp_s_and_preserves_accents() {
    assert_eq!(
        value("SELECT UPPER('weiß élève')"),
        ConstantValue::Text("WEISS ÉLÈVE".into())
    );
    assert_eq!(
        value("SELECT UPPER('é😀')"),
        ConstantValue::Text("É😀".into())
    );
}
