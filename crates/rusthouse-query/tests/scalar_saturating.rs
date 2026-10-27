use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn saturating_math_clamps_on_signed_overflow() {
    for (sql, expected) in [
        ("SELECT SATURATING_ADD(9223372036854775807, 1) AS result", "9223372036854775807"),
        ("SELECT saturating_sub(-9223372036854775807, 2) AS result", "-9223372036854775808"),
        ("SELECT SATURATING_MUL(9223372036854775807, 2) AS result", "9223372036854775807"),
        ("SELECT SATURATING_MUL(-9223372036854775807, 2) AS result", "-9223372036854775808"),
        ("SELECT SATURATING_ADD(3, -2) AS result", "1"),
        ("SELECT SATURATING_MUL(-4, -3) AS result", "12"),
    ] {
        assert_eq!(evaluate(sql).unwrap(), format!("result\n{expected}\n"), "{sql}");
    }
}

#[test]
fn saturating_math_rejects_bad_arity_and_table_sources() {
    for sql in [
        "SELECT SATURATING_ADD()",
        "SELECT SATURATING_SUB(1)",
        "SELECT SATURATING_MUL(1, 2, 3)",
        "SELECT SATURATING_ADD(1, 2",
        "SELECT SATURATING_ADD(1, 2) FROM numbers(1)",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
