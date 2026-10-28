use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn json_key_returns_field_name_by_one_based_ordinal() {
    for (sql, expected) in [
        (r#"SELECT JSONKey('{"one":1,"two":2}',1)"#, "one"),
        (r#"SELECT JSONKey('{"one":1,"two":2}',2)"#, "two"),
        (r#"SELECT JSONKey('{"nested":{"alpha":1,"beta":2}}','nested',2)"#, "beta"),
        (r#"SELECT JSONKey('{"a":42}',2)"#, ""),
        (r#"SELECT JSONKey('{"a":42}',0)"#, ""),
        (r#"SELECT JSONKey('{"a":[1,2]}','a',1)"#, ""),
        (r#"SELECT JSONKey('{"a":42}','missing',1)"#, ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT JSONKey(NULL, 1)"), ConstantValue::Null);
    assert_eq!(value("SELECT JSONKey('{\"a\":1}', NULL)"), ConstantValue::Null);
}
#[test]
fn json_key_rejects_invalid_json_or_arguments() {
    for sql in [
        "SELECT JSONKey('invalid', 1)",
        "SELECT JSONKey('{}')",
        "SELECT JSONKey('{}', 'a')",
        "SELECT JSONKey('{}', 1, 2)",
        "SELECT JSONKey('{}', 'a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
