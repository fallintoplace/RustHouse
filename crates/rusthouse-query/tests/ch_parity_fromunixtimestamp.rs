use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn evaluates_utc_parity_and_null_inputs() {
    assert_eq!(
        value("SELECT fromUnixTimestamp(1704067200)"),
        ConstantValue::Text("2024-01-01 00:00:00".into())
    );
    assert_eq!(
        value("SELECT fromUnixTimestamp(0)"),
        ConstantValue::Text("1970-01-01 00:00:00".into())
    );
    assert_eq!(
        value("SELECT fromUnixTimestamp(NULL)"),
        ConstantValue::Null
    );
}

#[test]
fn rejects_unsupported_types_ranges_and_arity() {
    for sql in [
        "SELECT fromUnixTimestamp()",
        "SELECT fromUnixTimestamp(-1)",
        "SELECT fromUnixTimestamp(4294967296)",
        "SELECT fromUnixTimestamp('bad')",
        "SELECT fromUnixTimestamp(0, 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
