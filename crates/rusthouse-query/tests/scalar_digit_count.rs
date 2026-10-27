use rusthouse_query::{Projection, parse_select};

#[test]
fn digit_count_counts_decimal_magnitude_digits() {
    for (sql, expected) in [
        ("SELECT DIGIT_COUNT(0)", 1),
        ("SELECT digit_count(9)", 1),
        ("SELECT DIGIT_COUNT(10)", 2),
        ("SELECT DIGIT_COUNT(-1000)", 4),
        ("SELECT DIGIT_COUNT(999999)", 6),
        ("SELECT DIGIT_COUNT(9223372036854775807)", 19),
        ("SELECT DIGIT_COUNT(-9223372036854775808)", 19),
        ("SELECT DIGIT_COUNT(12 * 10 + 1)", 3),
        ("SELECT DIGIT_COUNT(ABS(-1234))", 4),
        ("SELECT 1 + DIGIT_COUNT(99)", 3),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn digit_count_rejects_malformed_calls_and_source_queries() {
    for sql in [
        "SELECT DIGIT_COUNT()",
        "SELECT DIGIT_COUNT(1, 2)",
        "SELECT DIGIT_COUNT(1",
        "SELECT DIGIT_COUNT('bad')",
        "SELECT DIGIT_COUNT(1) FROM numbers(2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
