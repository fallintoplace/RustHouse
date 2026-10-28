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
        ("SELECT YYYYMMDDhhmmssToDateTime(20230911131415)", "2023-09-11 13:14:15"),
        ("SELECT YYYYMMDDhhmmssToDateTime(19700101000000)", "1970-01-01 00:00:00"),
        ("SELECT YYYYMMDDhhmmssToDateTime(20000229235959)", "2000-02-29 23:59:59"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT YYYYMMDDhhmmssToDateTime(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_numeric_calendar_values() {
    for sql in [
        "SELECT YYYYMMDDhhmmssToDateTime()",
        "SELECT YYYYMMDDhhmmssToDateTime(20230229123000)",
        "SELECT YYYYMMDDhhmmssToDateTime(202401021122)",
        "SELECT YYYYMMDDhhmmssToDateTime(-1)",
        "SELECT YYYYMMDDhhmmssToDateTime('2024-01-01')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
