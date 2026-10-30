use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        value => panic!("unexpected projection: {value:?}"),
    }
}

#[test]
fn leap_year_validates_utc_and_null() {
    for (value, expected) in [
        ("1900-01-01", 0),
        ("2000-01-01", 1),
        ("2024-12-31", 1),
        ("2025-01-01", 0),
    ] {
        let sql = format!("SELECT isLeapYear('{value}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT isLeapYear(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT isLeapYear('2024-02-30')").is_err());
    assert!(parse_select("SELECT isLeapYear('2024-01-01', 2)").is_err());
}
