use rusthouse_query::{Projection, parse_select};

#[test]
fn round_to_multiple_handles_signed_values_and_ties_away_from_zero() {
    for (sql, expected) in [
        ("SELECT ROUND_TO_MULTIPLE(12, 5)", 10),
        ("SELECT round_to_multiple(13, 5)", 15),
        ("SELECT ROUND_TO_MULTIPLE(-12, 5)", -10),
        ("SELECT ROUND_TO_MULTIPLE(-13, 5)", -15),
        ("SELECT ROUND_TO_MULTIPLE(15, 10)", 20),
        ("SELECT ROUND_TO_MULTIPLE(-15, 10)", -20),
        ("SELECT ROUND_TO_MULTIPLE(0, 7)", 0),
        ("SELECT ROUND_TO_MULTIPLE(9223372036854775807, 1)", i64::MAX),
        ("SELECT ROUND_TO_MULTIPLE(-9223372036854775808, 2)", i64::MIN),
        ("SELECT ROUND_TO_MULTIPLE(21 + 3, POWER(2, 3))", 24),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn round_to_multiple_rejects_zero_negative_steps_overflow_and_bad_arity() {
    for sql in [
        "SELECT ROUND_TO_MULTIPLE(5, 0)",
        "SELECT ROUND_TO_MULTIPLE(5, -2)",
        "SELECT ROUND_TO_MULTIPLE(9223372036854775807, 2)",
        "SELECT ROUND_TO_MULTIPLE(-9223372036854775808, 3)",
        "SELECT ROUND_TO_MULTIPLE()",
        "SELECT ROUND_TO_MULTIPLE(5)",
        "SELECT ROUND_TO_MULTIPLE(5, 2, 3)",
        "SELECT ROUND_TO_MULTIPLE(5, 2",
        "SELECT ROUND_TO_MULTIPLE('5', 2)",
        "SELECT ROUND_TO_MULTIPLE(5, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
