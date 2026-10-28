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
    assert_eq!(value("SELECT toMonthNumSinceEpoch('2024-10-01')"), ConstantValue::Integer(657));
    assert_eq!(value("SELECT toMonthNumSinceEpoch('1970-01-01')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT toMonthNumSinceEpoch(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_date_and_arity() {
    assert!(parse_select("SELECT toMonthNumSinceEpoch('2024-02-30')").is_err());
    assert!(parse_select("SELECT toMonthNumSinceEpoch()").is_err());
}
