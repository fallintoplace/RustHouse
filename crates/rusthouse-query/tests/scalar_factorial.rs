use rusthouse_query::{Projection, parse_select};

#[test]
fn factorial_evaluates_zero_boundaries_and_nested_expressions() {
    for (sql, expected) in [
        ("SELECT FACTORIAL(0)", 1),
        ("SELECT factorial(1)", 1),
        ("SELECT FACToRIAL(5)", 120),
        ("SELECT FACTORIAL(20)", 2_432_902_008_176_640_000),
        ("SELECT FACTORIAL(GCD(12, 8))", 24),
        ("SELECT FACTORIAL(3 + 1) / 2", 12),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn factorial_rejects_overflow_negative_and_invalid_calls() {
    for sql in [
        "SELECT FACTORIAL(-1)",
        "SELECT FACTORIAL(21)",
        "SELECT FACTORIAL(9223372036854775807)",
        "SELECT FACTORIAL()",
        "SELECT FACTORIAL(2, 3)",
        "SELECT FACTORIAL(5",
        "SELECT FACTORIAL('5')",
        "SELECT FACTORIAL(5) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
