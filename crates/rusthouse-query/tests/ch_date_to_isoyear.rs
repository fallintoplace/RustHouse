use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_isoyear_handles_calendar_boundaries_and_null() {
    assert_eq!(
        value("SELECT toISOYear('2021-01-01')"),
        ConstantValue::Integer(2020)
    );
    assert_eq!(value("SELECT toISOYear(NULL)"), ConstantValue::Null);
}

#[test]
fn to_isoyear_rejects_bad_dates_and_argument_shapes() {
    for sql in [
        "SELECT toISOYear('2023-02-29')",
        "SELECT toISOYear(1)",
        "SELECT toISOYear()",
        "SELECT toISOYear('2021-01-01', '2021-01-01')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
