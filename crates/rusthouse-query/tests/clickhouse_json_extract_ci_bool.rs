use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_case_insensitive_values() {
    assert_eq!(value("SELECT JSONExtractBoolCaseInsensitive('{\"ENABLED\":true}','enabled')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT JSONExtractBoolCaseInsensitive('{\"DATA\":{\"flag\":false}}','data','FLAG')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractBoolCaseInsensitive('{\"Flags\":[false,true]}','FLAGS',2)"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT JSONExtractBoolCaseInsensitive('{\"flags\":1}','FLAGS')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractBoolCaseInsensitive('{\"flags\":true}','missing')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONEXTRACTBOOLCASEINSENSITIVE(NULL,'key')"), ConstantValue::Null);
}

#[test]
fn invalid_json_fails_closed() {
    assert!(parse_select("SELECT JSONEXTRACTBOOLCASEINSENSITIVE('invalid', 'key')").is_err());
}
