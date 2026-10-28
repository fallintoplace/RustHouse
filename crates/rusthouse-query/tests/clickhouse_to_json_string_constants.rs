use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn json_encodes_constant_strings_and_null() {
    for (sql, expected) in [
        ("SELECT toJSONString('abc')", r#""abc""#),
        ("SELECT toJSONString('')", r#""""#),
        ("SELECT toJSONString('é')", r#""é""#),
        ("SELECT toJSONString(NULL)", "null"),
        ("SELECT toJSONString(JSON_QUOTE('a'))", r#""\"a\"""#),
    ] { assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}"); }
}
#[test]
fn existing_json_quote_null_semantics_stay_unchanged() {
    assert_eq!(value("SELECT JSON_QUOTE(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_wrong_arity_and_table_scans() {
    for sql in ["SELECT toJSONString()", "SELECT toJSONString('a', 'b')", "SELECT toJSONString('a') FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
