use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_year_accepts_valid_dates_and_null() {
    assert_eq!(
        value("SELECT toYear('2024-02-29')"),
        ConstantValue::Integer(2024)
    );
    assert_eq!(value("SELECT toYear(NULL)"), ConstantValue::Null);
}

#[test]
fn to_year_rejects_invalid_date_and_argument_shapes() {
    for sql in [
        "SELECT toYear('2023-02-29')",
        "SELECT toYear(1)",
        "SELECT toYear()",
        "SELECT toYear('2024-01-01', '2024-01-02')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
