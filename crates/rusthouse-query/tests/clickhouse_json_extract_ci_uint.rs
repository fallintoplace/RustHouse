use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_case_insensitive_values() {
    assert_eq!(value("SELECT JSONExtractUIntCaseInsensitive('{\"COUNT\":789}','count')"), ConstantValue::Integer(789));
    assert_eq!(value("SELECT JSONExtractUIntCaseInsensitive('{\"Data\":{\"N\":5}}','data','n')"), ConstantValue::Integer(5));
    assert_eq!(value("SELECT JSONExtractUIntCaseInsensitive('{\"Items\":[2,3]}','items',-1)"), ConstantValue::Integer(3));
    assert_eq!(value("SELECT JSONExtractUIntCaseInsensitive('{\"COUNT\":-1}','count')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractUIntCaseInsensitive('{\"count\":8}','missing')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONEXTRACTUINTCASEINSENSITIVE(NULL,'key')"), ConstantValue::Null);
}

#[test]
fn invalid_json_fails_closed() {
    assert!(parse_select("SELECT JSONEXTRACTUINTCASEINSENSITIVE('invalid', 'key')").is_err());
}
