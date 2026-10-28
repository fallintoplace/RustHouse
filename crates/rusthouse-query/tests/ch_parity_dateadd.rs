use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn handles_bounded_calendar_and_null_values() {
    assert_eq!(
        value("SELECT dateAdd('second', 1, '2024-02-29 23:59:59')"),
        ConstantValue::Text("2024-03-01 00:00:00".into())
    );
    assert_eq!(
        value("SELECT dateAdd('day', 1, '2024-02-28 00:00:00')"),
        ConstantValue::Text("2024-02-29 00:00:00".into())
    );
    assert_eq!(
        value("SELECT dateAdd('week', 1, '2024-02-29 12:00:00')"),
        ConstantValue::Text("2024-03-07 12:00:00".into())
    );
    assert_eq!(
        value("SELECT dateAdd('hour', -1, '2024-03-01 00:00:00')"),
        ConstantValue::Text("2024-02-29 23:00:00".into())
    );
    assert_eq!(
        value("SELECT dateAdd(NULL, NULL, NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_bad_calendar_types_and_ranges() {
    for sql in [
        "SELECT dateAdd()",
        "SELECT dateAdd('month', 1, '2024-01-01 00:00:00')",
        "SELECT dateAdd('hour', 1, 'bad')",
        "SELECT dateAdd('second', 1, '2106-02-07 06:28:15')",
        "SELECT dateAdd('day', 1, '2024-01-01 00:00:00', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
