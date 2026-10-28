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
    assert_eq!(value("SELECT addMonths('2024-01-31', 1)"), ConstantValue::Text("2024-02-29".into()));
    assert_eq!(value("SELECT addMonths('2024-03-31', -1)"), ConstantValue::Text("2024-02-29".into()));
    assert_eq!(value("SELECT addMonths(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_dates_and_overflow() {
    assert!(parse_select("SELECT addMonths('bad', 1)").is_err());
    assert!(parse_select("SELECT addMonths('2024-01-01')").is_err());
    assert!(parse_select("SELECT addMonths('2024-01-01', 9223372036854775807)").is_err());
}
