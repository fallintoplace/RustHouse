use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        result => panic!("unexpected projection: {result:?}"),
    }
}

#[test]
fn date_sub_handles_units_leap_days_and_nulls() {
    for (sql, expected) in [
        ("SELECT dateSub('day', 1, '2024-03-01 00:00:00')", "2024-02-29 00:00:00"),
        ("SELECT dateSub('hour', 2, '2024-03-01 01:30:00')", "2024-02-29 23:30:00"),
        ("SELECT dateSub('minute', -2, '2024-02-29 00:00:00')", "2024-02-29 00:02:00"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT dateSub('day', NULL, '2024-01-01')"), ConstantValue::Null);
    assert!(parse_select("SELECT dateSub('year', 2, '2024-01-01')").is_err());
    assert!(parse_select("SELECT dateSub('day', 1)").is_err());
}
