use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn handles_leap_calendar_boundaries_and_nulls() {
    assert_eq!(value("SELECT addYears('2020-02-29', 1)"), ConstantValue::Text("2021-02-28".into()));
    assert_eq!(value("SELECT addYears('2024-02-29', -1)"), ConstantValue::Text("2023-02-28".into()));
    assert_eq!(value("SELECT addYears(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_dates_and_overflow() {
    assert!(parse_select("SELECT addYears('bad', 1)").is_err());
    assert!(parse_select("SELECT addYears('2024-01-01')").is_err());
    assert!(parse_select("SELECT addYears('2024-01-01', 9223372036854775807)").is_err());
}
