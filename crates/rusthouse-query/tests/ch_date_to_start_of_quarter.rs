use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_start_of_quarter_handles_leap_days_and_null() {
    assert_eq!(
        value("SELECT toStartOfQuarter('2024-02-29')"),
        ConstantValue::Text("2024-01-01".into())
    );
    assert_eq!(value("SELECT toStartOfQuarter(NULL)"), ConstantValue::Null);
}

#[test]
fn to_start_of_quarter_rejects_invalid_dates_and_arity() {
    for sql in [
        "SELECT toStartOfQuarter('2023-02-29')",
        "SELECT toStartOfQuarter(1)",
        "SELECT toStartOfQuarter()",
        "SELECT toStartOfQuarter('2024-01-01', '2024-01-02')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
