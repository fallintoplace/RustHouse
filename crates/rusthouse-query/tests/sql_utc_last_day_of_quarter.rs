use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        value => panic!("unexpected projection: {value:?}"),
    }
}

#[test]
fn last_day_of_quarter_handles_calendar_boundaries() {
    for (value, expected) in [
        ("2024-02-29", "2024-03-31"),
        ("2024-04-15", "2024-06-30"),
        ("2024-12-01", "2024-12-31"),
    ] {
        let sql = format!("SELECT toLastDayOfQuarter('{value}')");
        assert_eq!(scalar(&sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(scalar("SELECT toLastDayOfQuarter(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT toLastDayOfQuarter('2024-02-30')").is_err());
    assert!(parse_select("SELECT toLastDayOfQuarter('2024-01-01', 2)").is_err());
}
