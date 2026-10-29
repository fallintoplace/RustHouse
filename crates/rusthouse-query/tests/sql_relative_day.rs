use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        value => panic!("unexpected projection: {value:?}"),
    }
}

#[test]
fn day_buckets_handle_utc_boundaries_and_null() {
    for (value, expected) in [
        ("1969-12-31", -1),
        ("1970-01-01", 0),
        ("1970-01-02", 1),
    ] {
        let sql = format!("SELECT toRelativeDayNum('{value}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT toRelativeDayNum(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT toRelativeDayNum('2024-02-30')").is_err());
    assert!(parse_select("SELECT toRelativeDayNum('2024-01-01', 2)").is_err());
}
