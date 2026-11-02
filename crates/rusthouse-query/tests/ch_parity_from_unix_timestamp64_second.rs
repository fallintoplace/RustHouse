use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn renders_signed_unix_seconds_as_utc() {
    for (sql, expected) in [
        ("SELECT fromUnixTimestamp64Second(0)", "1970-01-01 00:00:00"),
        ("SELECT fromUnixTimestamp64Second(-1)", "1969-12-31 23:59:59"),
        ("SELECT fromUnixTimestamp64Second(1704067200)", "2024-01-01 00:00:00"),
        ("SELECT fromUnixTimestamp64Second(951782400)", "2000-02-29 00:00:00"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT fromUnixTimestamp64Second(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_bad_arity_and_out_of_range_date() {
    for sql in [
        "SELECT fromUnixTimestamp64Second()",
        "SELECT fromUnixTimestamp64Second(0, 1)",
        "SELECT fromUnixTimestamp64Second(9223372036854775807)",
        "SELECT fromUnixTimestamp64Second('2024-01-01')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
