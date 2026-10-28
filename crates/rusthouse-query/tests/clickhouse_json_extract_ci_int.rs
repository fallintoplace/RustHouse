use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_case_insensitive_values() {
    assert_eq!(value("SELECT JSONExtractIntCaseInsensitive('{\"Value\":123}','value')"), ConstantValue::Integer(123));
    assert_eq!(value("SELECT JSONExtractIntCaseInsensitive('{\"DATA\":{\"COUNT\":42}}','data','Count')"), ConstantValue::Integer(42));
    assert_eq!(value("SELECT JSONExtractIntCaseInsensitive('{\"items\":[11,22]}','ITEMS',2)"), ConstantValue::Integer(22));
    assert_eq!(value("SELECT JSONExtractIntCaseInsensitive('{\"value\":1.5}','VALUE')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractIntCaseInsensitive('{\"value\":1}','missing')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONEXTRACTINTCASEINSENSITIVE(NULL,'key')"), ConstantValue::Null);
}

#[test]
fn invalid_json_fails_closed() {
    assert!(parse_select("SELECT JSONEXTRACTINTCASEINSENSITIVE('invalid', 'key')").is_err());
}
