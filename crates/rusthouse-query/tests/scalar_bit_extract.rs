use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn bit_extract_reads_unsigned_fields_from_signed_inputs() {
    for (sql, expected) in [
        ("SELECT BIT_EXTRACT(15, 1, 3) AS result", "7"),
        ("SELECT bit_extract(-1, 63, 1) AS result", "1"),
        ("SELECT BIT_EXTRACT(-2, 1, 4) AS result", "15"),
        ("SELECT BIT_EXTRACT(0, 0, 1) AS result", "0"),
        ("SELECT BIT_EXTRACT(-1, 0, 64) AS result", "-1"),
        ("SELECT BIT_EXTRACT(3, 0, 64) AS result", "3"),
        (
            "SELECT BIT_EXTRACT(9223372036854775807, 1, 63) AS result",
            "4611686018427387903",
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
fn bit_extract_rejects_fields_outside_the_word() {
    for sql in [
        "SELECT BIT_EXTRACT(3, -1, 1)",
        "SELECT BIT_EXTRACT(3, 64, 1)",
        "SELECT BIT_EXTRACT(3, 0, 0)",
        "SELECT BIT_EXTRACT(3, 1, 64)",
        "SELECT BIT_EXTRACT(3, 63, 2)",
        "SELECT BIT_EXTRACT(3, 0)",
        "SELECT BIT_EXTRACT(3, 0, 1, 2)",
        "SELECT BIT_EXTRACT(3, 0, 1) FROM numbers(1)",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
