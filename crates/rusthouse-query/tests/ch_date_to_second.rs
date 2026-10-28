use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_second_handles_utc_datetime_and_null() {
    assert_eq!(
        value("SELECT toSecond('2024-02-29 23:45:09')"),
        ConstantValue::Integer(9)
    );
    assert_eq!(value("SELECT toSecond(NULL)"), ConstantValue::Null);
}

#[test]
fn to_second_rejects_invalid_datetime_and_arity() {
    for sql in [
        "SELECT toSecond('2023-02-29 23:45:09')",
        "SELECT toSecond(1)",
        "SELECT toSecond()",
        "SELECT toSecond('2024-02-29 23:45:09', '2024-02-29 23:45:09')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
