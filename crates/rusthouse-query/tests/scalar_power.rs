use rusthouse_query::{execute, parse_select};

fn query(sql: &str) -> Result<String, String> {
    let statement = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&statement, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn power_supports_checked_integer_exponents() {
    assert_eq!(query("SELECT POWER(2, 10)").unwrap(), "1024\n1024\n");
    assert_eq!(query("SELECT POW(3, 0)").unwrap(), "1\n1\n");
    assert_eq!(query("SELECT POWER(2, 1 / 2)").unwrap(), "1\n1\n");
    assert_eq!(
        query("SELECT POWER(-2, 63) AS result").unwrap(),
        "result\n-9223372036854775808\n"
    );
    assert_eq!(query("SELECT 1 + POWER(2, 3 * 2)").unwrap(), "65\n65\n");
    assert_eq!(
        query("SELECT POWER(2, 3), 'ok'").unwrap(),
        "8\t'ok'\n8\tok\n"
    );
}

#[test]
fn power_rejects_overflow_negative_exponents_and_incomplete_arguments() {
    for sql in [
        "SELECT POWER(2, 63)",
        "SELECT POWER(2, -1)",
        "SELECT POWER(2, 4294967296)",
        "SELECT POWER(2)",
        "SELECT POWER(2, 3, 4)",
        "SELECT POWER(2, 3",
    ] {
        assert!(query(sql).is_err(), "{sql}");
    }
}
