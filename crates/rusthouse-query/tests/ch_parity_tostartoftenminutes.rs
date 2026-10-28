use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn rounds_utc_time_down_and_preserves_null() {
    assert_eq!(
        value("SELECT toStartOfTenMinutes('2024-02-29 23:59:59')"),
        ConstantValue::Text("2024-02-29 23:50:00".into())
    );
    assert_eq!(
        value("SELECT toStartOfTenMinutes('1970-01-01 00:00:00')"),
        ConstantValue::Text("1970-01-01 00:00:00".into())
    );
    assert_eq!(value("SELECT toStartOfTenMinutes(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_types_ranges_and_arity() {
    for sql in [
        "SELECT toStartOfTenMinutes()",
        "SELECT toStartOfTenMinutes(17)",
        "SELECT toStartOfTenMinutes('2023-02-29 23:59:59')",
        "SELECT toStartOfTenMinutes('2024-02-29 23:59:59', 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
