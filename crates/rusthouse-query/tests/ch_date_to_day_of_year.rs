use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_day_of_year_accepts_leap_days_and_null() {
    assert_eq!(
        value("SELECT toDayOfYear('2024-02-29')"),
        ConstantValue::Integer(60)
    );
    assert_eq!(value("SELECT toDayOfYear(NULL)"), ConstantValue::Null);
}

#[test]
fn to_day_of_year_rejects_invalid_dates_and_arity() {
    for sql in [
        "SELECT toDayOfYear('2023-02-29')",
        "SELECT toDayOfYear(123)",
        "SELECT toDayOfYear()",
        "SELECT toDayOfYear('2024-01-01', '2024-01-02')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
