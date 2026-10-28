use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_clickhouse_cases() {
    assert_eq!(value("SELECT JSONExtractUInt('{\"n\":42}', 'n')"), ConstantValue::Integer(42));
    assert_eq!(value("SELECT JSONExtractUInt('{\"n\":-42}', 'n')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractUInt('{\"n\":1.5}', 'n')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractUInt('{\"a\":{\"n\":7}}','a.n')"), ConstantValue::Integer(7));
    assert_eq!(value("SELECT JSONExtractUInt('{\"n\":42}','missing')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT JSONExtractUInt(NULL,'n')"), ConstantValue::Null);
}

#[test]
fn rejects_unsupported_or_malformed_calls() {
    for sql in [
        "SELECT JSONExtractUInt('invalid')",
        "SELECT JSONExtractUInt('{\"n\":9223372036854775808}', 'n')",
        "SELECT JSONExtractUInt('{}', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
