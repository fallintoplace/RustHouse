use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn bit_reverse_preserves_all_sixty_four_bits() {
    for (sql, expected) in [
        ("SELECT BIT_REVERSE(0) AS result", "0"),
        ("SELECT bit_reverse(-1) AS result", "-1"),
        ("SELECT BIT_REVERSE(1) AS result", "-9223372036854775808"),
        ("SELECT BIT_REVERSE(2) AS result", "4611686018427387904"),
        (
            "SELECT BIT_REVERSE(-9223372036854775807 - 1) AS result",
            "1",
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
fn bit_reverse_rejects_bad_arguments_and_table_source() {
    for sql in [
        "SELECT BIT_REVERSE()",
        "SELECT BIT_REVERSE(1, 2)",
        "SELECT BIT_REVERSE(1",
        "SELECT BIT_REVERSE(1) FROM numbers(2)",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
