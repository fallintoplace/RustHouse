use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_day_of_month_accepts_valid_dates_and_null() {
    assert_eq!(
        value("SELECT toDayOfMonth('2024-02-29')"),
        ConstantValue::Integer(29)
    );
    assert_eq!(value("SELECT toDayOfMonth(NULL)"), ConstantValue::Null);
}

#[test]
fn to_day_of_month_rejects_invalid_date_and_argument_shapes() {
    for sql in [
        "SELECT toDayOfMonth('2023-02-29')",
        "SELECT toDayOfMonth(1)",
        "SELECT toDayOfMonth()",
        "SELECT toDayOfMonth('2024-01-01', '2024-01-02')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
