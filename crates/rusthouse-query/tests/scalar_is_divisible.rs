use rusthouse_query::{Projection, parse_select};

#[test]
fn scalar_is_divisible_supports_signed_and_composed_constants() {
    for (sql, expected) in [
        ("SELECT IS_DIVISIBLE(12, 3)", 1),
        ("SELECT is_divisible(12, 5)", 0),
        ("SELECT IS_DIVISIBLE(-12, 3)", 1),
        ("SELECT IS_DIVISIBLE(12, -3)", 1),
        ("SELECT IS_DIVISIBLE(0, -3)", 1),
        ("SELECT IS_DIVISIBLE(-9223372036854775808, -1)", 1),
        ("SELECT IS_DIVISIBLE(3 * 4, 5) + IS_DIVISIBLE(12, 4)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn scalar_is_divisible_rejects_invalid_arguments() {
    for sql in [
        "SELECT IS_DIVISIBLE(1, 0)",
        "SELECT IS_DIVISIBLE(1)",
        "SELECT IS_DIVISIBLE(1, 2, 3)",
        "SELECT IS_DIVISIBLE('x', 2)",
        "SELECT IS_DIVISIBLE(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
