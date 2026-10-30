use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn division_is_signed_truncating_and_has_multiplicative_precedence() {
    assert_eq!(evaluate("SELECT 7 / 2 AS q").unwrap(), "q\n3\n");
    assert_eq!(evaluate("SELECT -7 / 2").unwrap(), "-3\n-3\n");
    assert_eq!(evaluate("SELECT 7 / -2").unwrap(), "-3\n-3\n");
    assert_eq!(evaluate("SELECT 8 / 2 % 3").unwrap(), "1\n1\n");
    assert_eq!(evaluate("SELECT -9223372036854775808 % -1 AS rem").unwrap(), "rem\n0\n");
    assert!(evaluate("SELECT 2 % 0").is_err());
    assert_eq!(evaluate("SELECT 1 + 10 / (2 + 3)").unwrap(), "3\n3\n");
}

#[test]
fn division_reports_zero_overflow_and_incomplete_expressions() {
    for sql in [
        "SELECT 8 / 0",
        "SELECT -9223372036854775808 / -1",
        "SELECT 1 /",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}

#[test]
fn subtraction_works_without_whitespace_and_after_parentheses() {
    assert_eq!(evaluate("SELECT 10-2 * 3").unwrap(), "4\n4\n");
    assert_eq!(evaluate("SELECT (10)- -3").unwrap(), "13\n13\n");
}
