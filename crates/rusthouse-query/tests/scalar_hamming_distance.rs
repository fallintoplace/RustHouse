use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn hamming_distance_counts_differing_twos_complement_bits() {
    for (sql, expected) in [
        ("SELECT HAMMING_DISTANCE(5, 1) AS result", "1"),
        ("SELECT hamming_distance(-1, 0) AS result", "64"),
        ("SELECT HAMMING_DISTANCE(-1, -2) AS result", "1"),
        ("SELECT HAMMING_DISTANCE(0, 0) AS result", "0"),
        ("SELECT HAMMING_DISTANCE(0, 9223372036854775807) AS result", "63"),
        ("SELECT HAMMING_DISTANCE(3 + 2, 2 * 2) AS result", "1"),
    ] {
        assert_eq!(evaluate(sql).unwrap(), format!("result\n{expected}\n"), "{sql}");
    }
}

#[test]
fn hamming_distance_rejects_bad_arity_and_from_clause() {
    for sql in [
        "SELECT HAMMING_DISTANCE()",
        "SELECT HAMMING_DISTANCE(1)",
        "SELECT HAMMING_DISTANCE(1, 2, 3)",
        "SELECT HAMMING_DISTANCE(1, 2",
        "SELECT HAMMING_DISTANCE(1, 2) FROM numbers(1)",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
