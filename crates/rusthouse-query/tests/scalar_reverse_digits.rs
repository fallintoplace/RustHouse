use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn reverse_digits_preserves_sign_and_drops_trailing_zeros() {
    for (sql, expected) in [
        ("SELECT REVERSE_DIGITS(123450) AS result", "54321"),
        ("SELECT reverse_digits(-120) AS result", "-21"),
        ("SELECT REVERSE_DIGITS(0) AS result", "0"),
        ("SELECT REVERSE_DIGITS(1000) AS result", "1"),
        (
            "SELECT REVERSE_DIGITS(-8085774586302733229) AS result",
            "-9223372036854775808",
        ),
    ] {
        assert_eq!(
            evaluate(sql).unwrap(),
            format!("result\n{expected}\n"),
            "{sql}"
        );
    }
}

#[test]
fn reverse_digits_rejects_overflow_and_bad_inputs() {
    for sql in [
        "SELECT REVERSE_DIGITS(8085774586302733229)",
        "SELECT REVERSE_DIGITS()",
        "SELECT REVERSE_DIGITS(1, 2)",
        "SELECT REVERSE_DIGITS(1",
        "SELECT REVERSE_DIGITS(1) FROM numbers(1)",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
