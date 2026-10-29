use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        value => panic!("unexpected projection: {value:?}"),
    }
}

#[test]
fn week_buckets_handle_utc_boundaries_and_null() {
    for (value, expected) in [
        ("1969-12-27", -1),
        ("1969-12-28", 0),
        ("1970-01-03", 0),
        ("1970-01-04", 1),
    ] {
        let sql = format!("SELECT toRelativeWeekNum('{value}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT toRelativeWeekNum(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT toRelativeWeekNum('2024-02-30')").is_err());
    assert!(parse_select("SELECT toRelativeWeekNum('2024-01-01', 2)").is_err());
}
