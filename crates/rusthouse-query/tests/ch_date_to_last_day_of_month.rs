use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_last_day_of_month_handles_leap_days_and_null() {
    assert_eq!(
        value("SELECT toLastDayOfMonth('2024-02-29')"),
        ConstantValue::Text("2024-02-29".into())
    );
    assert_eq!(value("SELECT toLastDayOfMonth(NULL)"), ConstantValue::Null);
}

#[test]
fn to_last_day_of_month_rejects_invalid_dates_and_arity() {
    for sql in [
        "SELECT toLastDayOfMonth('2023-02-29')",
        "SELECT toLastDayOfMonth(1)",
        "SELECT toLastDayOfMonth()",
        "SELECT toLastDayOfMonth('2024-01-01', '2024-01-02')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
