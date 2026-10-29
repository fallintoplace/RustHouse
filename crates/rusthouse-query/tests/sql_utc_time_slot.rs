use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) => values[0].value.clone(),
        value => panic!("unexpected projection: {value:?}"),
    }
}

#[test]
fn time_slot_validates_utc_and_null() {
    for (value, expected) in [
        ("1969-12-31 23:59:59", "1969-12-31 23:30:00"),
        ("1970-01-01 00:29:59", "1970-01-01 00:00:00"),
        ("2024-02-29 12:32:00", "2024-02-29 12:30:00"),
    ] {
        let sql = format!("SELECT timeSlot('{value}')");
        assert_eq!(scalar(&sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(scalar("SELECT timeSlot(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT timeSlot('2024-02-30')").is_err());
    assert!(parse_select("SELECT timeSlot('2024-01-01', 2)").is_err());
}
