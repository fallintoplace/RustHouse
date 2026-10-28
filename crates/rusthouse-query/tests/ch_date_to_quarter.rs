use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_quarter_accepts_valid_dates_and_null() {
    assert_eq!(
        value("SELECT toQuarter('2024-02-29')"),
        ConstantValue::Integer(1)
    );
    assert_eq!(value("SELECT toQuarter(NULL)"), ConstantValue::Null);
}

#[test]
fn to_quarter_rejects_invalid_date_and_argument_shapes() {
    for sql in [
        "SELECT toQuarter('2023-02-29')",
        "SELECT toQuarter(1)",
        "SELECT toQuarter()",
        "SELECT toQuarter('2024-01-01', '2024-01-02')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
