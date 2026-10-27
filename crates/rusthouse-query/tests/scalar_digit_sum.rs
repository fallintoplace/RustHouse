use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn digit_sum_supports_zero_negative_values_and_minimum_int64() {
    for (sql, expected) in [
        ("SELECT DIGIT_SUM(12345) AS result", "15"),
        ("SELECT digit_sum(-909) AS result", "18"),
        ("SELECT DIGIT_SUM(0) AS result", "0"),
        ("SELECT DIGIT_SUM(-9223372036854775807 - 1) AS result", "89"),
        ("SELECT DIGIT_SUM(12 + 34) AS result", "10"),
    ] {
        assert_eq!(
            evaluate(sql).unwrap(),
            format!("result\n{expected}\n"),
            "{sql}"
        );
    }
}

#[test]
fn digit_sum_rejects_invalid_arity_and_table_source() {
    for sql in [
        "SELECT DIGIT_SUM()",
        "SELECT DIGIT_SUM(1, 2)",
        "SELECT DIGIT_SUM(1",
        "SELECT DIGIT_SUM(1) FROM numbers(1)",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
