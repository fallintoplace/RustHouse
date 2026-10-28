use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_yyyymm_accepts_leap_days_and_null() {
    assert_eq!(
        value("SELECT toYYYYMM('2024-02-29')"),
        ConstantValue::Integer(202402)
    );
    assert_eq!(value("SELECT toYYYYMM(NULL)"), ConstantValue::Null);
}

#[test]
fn to_yyyymm_rejects_invalid_dates_and_arity() {
    for sql in [
        "SELECT toYYYYMM('2023-02-29')",
        "SELECT toYYYYMM(123)",
        "SELECT toYYYYMM()",
        "SELECT toYYYYMM('2024-01-01', '2024-01-02')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
