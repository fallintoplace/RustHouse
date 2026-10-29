use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        value => panic!("unexpected projection: {value:?}"),
    }
}

#[test]
fn days_in_month_validates_utc_and_null() {
    for (value, expected) in [
        ("2024-02-01", 29),
        ("1900-02-28", 28),
        ("2000-02-29", 29),
        ("2025-04-30", 30),
    ] {
        let sql = format!("SELECT daysInMonth('{value}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT daysInMonth(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT daysInMonth('2024-02-30')").is_err());
    assert!(parse_select("SELECT daysInMonth('2024-01-01', 2)").is_err());
}
