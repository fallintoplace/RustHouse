use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn formats_bounded_epoch_ticks_in_utc() {
    for (sql, expected) in [
        ("SELECT fromUnixTimestamp64Milli(1_640_995_200_123)", "2022-01-01 00:00:00.123"),
        ("SELECT fromUnixTimestamp64Milli(0)", "1970-01-01 00:00:00.000"),
        ("SELECT fromUnixTimestamp64Milli(1)", "1970-01-01 00:00:00.001"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT fromUnixTimestamp64Milli(NULL)"), ConstantValue::Null);
}

#[test]
fn invalid_arguments_and_date_range_are_rejected() {
    for sql in [
        "SELECT fromUnixTimestamp64Milli()",
        "SELECT fromUnixTimestamp64Milli(-1)",
        "SELECT fromUnixTimestamp64Milli('2022-01-01')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
