use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        value => panic!("unexpected projection: {value:?}"),
    }
}

#[test]
fn to_week_handles_calendar_boundaries() {
    for (value, expected) in [
        ("2021-01-01", 0),
        ("2021-01-03", 1),
        ("2017-01-01", 1),
        ("2016-12-31", 52),
    ] {
        let sql = format!("SELECT toWeek('{value}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT toWeek(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT toWeek('2024-02-30')").is_err());
    assert!(parse_select("SELECT toWeek('2024-01-01', 2)").is_err());
}
