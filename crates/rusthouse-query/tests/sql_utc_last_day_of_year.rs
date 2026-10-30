use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        value => panic!("unexpected projection: {value:?}"),
    }
}

#[test]
fn last_day_of_year_handles_calendar_boundaries() {
    for (value, expected) in [
        ("1900-02-01", "1900-12-31"),
        ("2024-02-29", "2024-12-31"),
        ("2299-01-01", "2299-12-31"),
    ] {
        let sql = format!("SELECT toLastDayOfYear('{value}')");
        assert_eq!(scalar(&sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(scalar("SELECT toLastDayOfYear(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT toLastDayOfYear('2024-02-30')").is_err());
    assert!(parse_select("SELECT toLastDayOfYear('2024-01-01', 2)").is_err());
}
