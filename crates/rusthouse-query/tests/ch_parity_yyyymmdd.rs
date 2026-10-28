use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn packed_calendar_numbers_convert_to_utc() {
    for (sql, expected) in [
        ("SELECT YYYYMMDDToDate(20240115)", "2024-01-15"),
        ("SELECT YYYYMMDDToDate(19700101)", "1970-01-01"),
        ("SELECT YYYYMMDDToDate(20000229)", "2000-02-29"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT YYYYMMDDToDate(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_numeric_calendar_values() {
    for sql in [
        "SELECT YYYYMMDDToDate()",
        "SELECT YYYYMMDDToDate(20230229)",
        "SELECT YYYYMMDDToDate(202401)",
        "SELECT YYYYMMDDToDate(-1)",
        "SELECT YYYYMMDDToDate('2024-01-01')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
