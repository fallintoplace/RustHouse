use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn fibonacci_returns_exact_sequence_values() {
    for (sql, expected) in [
        ("SELECT FIBONACCI(0) AS result", "0"),
        ("SELECT fibonacci(1) AS result", "1"),
        ("SELECT FIBONACCI(2) AS result", "1"),
        ("SELECT FIBONACCI(10) AS result", "55"),
        ("SELECT FIBONACCI(92) AS result", "7540113804746346429"),
    ] {
        assert_eq!(evaluate(sql).unwrap(), format!("result\n{expected}\n"), "{sql}");
    }
}

#[test]
fn fibonacci_rejects_out_of_range_indices_and_arity() {
    for sql in [
        "SELECT FIBONACCI(-1)",
        "SELECT FIBONACCI(93)",
        "SELECT FIBONACCI()",
        "SELECT FIBONACCI(1, 2)",
        "SELECT FIBONACCI(1) FROM numbers(1)",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
