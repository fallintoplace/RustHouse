use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn handles_calendar_boundaries_and_nulls() {
    assert_eq!(value("SELECT dateDiff('day', '2024-02-28', '2024-03-01')"), ConstantValue::Integer(2));
    assert_eq!(value("SELECT dateDiff('month', '2023-12-31', '2024-01-01')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT dateDiff('year', '2023-12-31', '2024-01-01')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT dateDiff('day', NULL, '2024-02-29')"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_dates_and_arity() {
    assert!(parse_select("SELECT dateDiff('day', 'bad', '2024-02-29')").is_err());
    assert!(parse_select("SELECT dateDiff('day', '2024-01-01')").is_err());
}
