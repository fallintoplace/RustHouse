use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn handles_date_boundaries_and_nulls() {
    assert_eq!(value("SELECT toUnixTimestamp('1970-01-01T01:00:00+01:00')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toUnixTimestamp('1970-01-01 00:00:01')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT toUnixTimestamp(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_date_and_arity() {
    assert!(parse_select("SELECT toUnixTimestamp('2024-02-30 25:17:42')").is_err());
    assert!(parse_select("SELECT toUnixTimestamp()").is_err());
}
