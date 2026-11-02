use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_clickhouse_date_name_components() {
    for (part, expected) in [
        ("year", "2021"), ("quarter", "2"), ("month", "April"),
        ("week", "15"), ("dayofyear", "104"), ("day", "14"),
        ("weekday", "Wednesday"), ("hour", "11"),
        ("minute", "22"), ("second", "33"),
    ] {
        let sql = format!("SELECT dateName('{part}', '2021-04-14 11:22:33')");
        assert_eq!(value(&sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT dateName('weekday', '1970-01-01')"),
        ConstantValue::Text("Thursday".into()),
    );
    assert_eq!(
        value("SELECT dateName('month', '2024-02-29', 'UTC')"),
        ConstantValue::Text("February".into()),
    );
}

#[test]
fn rejects_invalid_date_parts_and_non_utc_timezones() {
    assert_eq!(value("SELECT dateName(NULL, '2024-01-01')"), ConstantValue::Null);
    assert_eq!(value("SELECT dateName('month', NULL)"), ConstantValue::Null);
    for sql in [
        "SELECT dateName('nonsense', '2024-01-01')",
        "SELECT dateName('month', '2024-02-30')",
        "SELECT dateName('month', '2024-01-01', 'Europe/Berlin')",
        "SELECT dateName('month')",
        "SELECT dateName('month', '2024-01-01', 'UTC', 'UTC')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
