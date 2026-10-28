use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_case_insensitive_values() {
    assert_eq!(value("SELECT JSONExtractRawCaseInsensitive('{\"Object\":{\"key\":\"value\"}}','OBJECT')"), ConstantValue::Text("{\"key\":\"value\"}".into()));
    assert_eq!(value("SELECT JSONExtractRawCaseInsensitive('{\"Items\":[1,2]}','items',-1)"), ConstantValue::Text("2".into()));
    assert_eq!(value("SELECT JSONExtractRawCaseInsensitive('{\"items\":[1,2]}','ITEMS',1)"), ConstantValue::Text("1".into()));
    assert_eq!(value("SELECT JSONExtractRawCaseInsensitive('{\"A\":null}','a')"), ConstantValue::Text("null".into()));
    assert_eq!(value("SELECT JSONExtractRawCaseInsensitive('{\"A\":1}','missing')"), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT JSONEXTRACTRAWCASEINSENSITIVE(NULL,'key')"), ConstantValue::Null);
}

#[test]
fn invalid_json_fails_closed() {
    assert!(parse_select("SELECT JSONEXTRACTRAWCASEINSENSITIVE('invalid', 'key')").is_err());
}
