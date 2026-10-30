use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        value => panic!("unexpected projection: {value:?}"),
    }
}

#[test]
fn quarter_buckets_handle_utc_boundaries_and_null() {
    for (value, expected) in [
        ("1969-12-31", -1),
        ("1970-01-01", 0),
        ("1970-04-01", 1),
    ] {
        let sql = format!("SELECT toRelativeQuarterNum('{value}')");
        assert_eq!(scalar(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(scalar("SELECT toRelativeQuarterNum(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT toRelativeQuarterNum('2024-02-30')").is_err());
    assert!(parse_select("SELECT toRelativeQuarterNum('2024-01-01', 2)").is_err());
}
