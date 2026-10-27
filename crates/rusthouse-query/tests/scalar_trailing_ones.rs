use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn trailing_ones_counts_signed_bit_patterns() {
    for (sql, expected) in [
        ("SELECT TRAILING_ONES(0) AS result", "0"),
        ("SELECT trailing_ones(7) AS result", "3"),
        ("SELECT TRAILING_ONES(15) AS result", "4"),
        ("SELECT TRAILING_ONES(6) AS result", "0"),
        ("SELECT TRAILING_ONES(-1) AS result", "64"),
        ("SELECT TRAILING_ONES(-2) AS result", "0"),
    ] {
        assert_eq!(evaluate(sql).unwrap(), format!("result\n{expected}\n"), "{sql}");
    }
}

#[test]
fn trailing_ones_rejects_bad_arity_and_table_sources() {
    for sql in [
        "SELECT TRAILING_ONES()",
        "SELECT TRAILING_ONES(1, 2)",
        "SELECT TRAILING_ONES(1",
        "SELECT TRAILING_ONES(1) FROM numbers(1)",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
