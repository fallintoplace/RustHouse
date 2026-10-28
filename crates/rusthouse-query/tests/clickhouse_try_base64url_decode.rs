use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn decodes_padded_unpadded_whitespace_and_both_alphabets() {
    for (sql, expected) in [
        ("SELECT tryBase64URLDecode('SGVsbG8=')", "Hello"),
        ("SELECT tryBase64URLDecode('SGVsbG8')", "Hello"),
        ("SELECT tryBase64URLDecode('YQ')", "a"),
        ("SELECT tryBase64URLDecode('YQ==')", "a"),
        ("SELECT tryBase64URLDecode('Y Q==')", "a"),
        ("SELECT tryBase64URLDecode('')", ""),
    ] { assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}"); }
}
#[test]
fn returns_empty_on_invalid_base64_or_invalid_utf8() {
    for sql in [
        "SELECT tryBase64URLDecode('0!')",
        "SELECT tryBase64URLDecode('A')",
        "SELECT tryBase64URLDecode('=YQ')",
        "SELECT tryBase64URLDecode('YQ===')",
        "SELECT tryBase64URLDecode('_w==')",
    ] { assert_eq!(value(sql), ConstantValue::Text(String::new()), "{sql}"); }
    assert_eq!(value("SELECT tryBase64URLDecode(NULL)"), ConstantValue::Null);
}
#[test]
fn refuses_bad_arity_and_table_expression() {
    for sql in [
        "SELECT tryBase64URLDecode()",
        "SELECT tryBase64URLDecode('YQ', 'Yg')",
        "SELECT tryBase64URLDecode('YQ') FROM numbers(1)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
