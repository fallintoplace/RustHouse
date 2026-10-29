use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        value => panic!("unexpected projection: {value:?}"),
    }
}

#[test]
fn minute_buckets_handle_utc_boundaries_and_null() {
    for (value, expected) in [
        ("1969-12-31 23:59:00", -1),
        ("1970-01-01 00:00:59", 0),
        ("1970-01-01 00:01:00", 1),
    ] {
        let sql = format!("SELECT toRelativeMinuteNum('{value}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT toRelativeMinuteNum(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT toRelativeMinuteNum('2024-02-30')").is_err());
    assert!(parse_select("SELECT toRelativeMinuteNum('2024-01-01', 2)").is_err());
}
