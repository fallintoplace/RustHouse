use rusthouse_query::{ConstantValue, Projection, parse_select};

#[test]
fn add_days_handles_leap_dates_and_utc_time() {
    for (sql, expected) in [
        ("SELECT addDays('2024-02-28', 1)", "2024-02-29"),
        ("SELECT addDays('2024-12-31', 1)", "2025-01-01"),
        ("SELECT addDays('2024-03-01', -1)", "2024-02-29"),
        ("SELECT addDays('1970-01-01 23:59:59', 1)", "1970-01-02 23:59:59"),
    ] {
        let select = parse_select(sql).unwrap();
        let Projection::Constants(columns) = select.projection else {
            panic!("expected constant expression for {sql}");
        };
        assert_eq!(columns[0].value, ConstantValue::Text(expected.into()), "{sql}");
    }
    let Projection::Constants(columns) =
        parse_select("SELECT addDays(NULL, 1)").unwrap().projection
    else {
        panic!("expected nullable constant");
    };
    assert_eq!(columns[0].value, ConstantValue::Null);
}

#[test]
fn add_days_rejects_invalid_dates_and_arity() {
    for sql in [
        "SELECT addDays('2023-02-29', 1)",
        "SELECT addDays('2024-13-01', 1)",
        "SELECT addDays('2024-01-01 24:00:00', 1)",
        "SELECT addDays('2024-01-01')",
        "SELECT addDays('2024-01-01', 1, 2)",
        "SELECT addDays('2299-12-31', 1)",
        "SELECT addDays('2024-01-01', 'one')",
        "SELECT addDays('2024-01-01', 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
