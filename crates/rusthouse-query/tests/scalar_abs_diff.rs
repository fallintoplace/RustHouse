use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn abs_diff_uses_wide_intermediates_for_signed_arguments() {
    for (sql, expected) in [
        ("SELECT ABS_DIFF(-7, 4) AS result", "11"),
        ("SELECT abs_diff(4, -7) AS result", "11"),
        ("SELECT ABS_DIFF(5, 5) AS result", "0"),
        (
            "SELECT ABS_DIFF(-9223372036854775807, 0) AS result",
            "9223372036854775807",
        ),
        ("SELECT ABS_DIFF(-10, -20) AS result", "10"),
    ] {
        assert_eq!(
            evaluate(sql).unwrap(),
            format!("result\n{expected}\n"),
            "{sql}"
        );
    }
}

#[test]
fn abs_diff_rejects_overflow_and_bad_arguments() {
    for sql in [
        "SELECT ABS_DIFF(-9223372036854775807, 9223372036854775807)",
        "SELECT ABS_DIFF()",
        "SELECT ABS_DIFF(1)",
        "SELECT ABS_DIFF(1, 2, 3)",
        "SELECT ABS_DIFF(1, 2",
        "SELECT ABS_DIFF(1, 2) FROM numbers(1)",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
