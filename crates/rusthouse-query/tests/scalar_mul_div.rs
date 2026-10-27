use rusthouse_query::{Projection, parse_select};

#[test]
fn mul_div_uses_wide_intermediate_and_truncates_toward_zero() {
    for (sql, expected) in [
        ("SELECT MUL_DIV(6, 7, 3)", 14),
        ("SELECT mul_div(-7, 3, 2)", -10),
        ("SELECT MUL_DIV(7, 3, -2)", -10),
        ("SELECT MUL_DIV(-7, 3, -2)", 10),
        ("SELECT MUL_DIV(0, -2, 3)", 0),
        ("SELECT MUL_DIV(9223372036854775807, 2, 2)", i64::MAX),
        (
            "SELECT MUL_DIV(9223372036854775807, 9223372036854775807, 9223372036854775807)",
            i64::MAX,
        ),
        ("SELECT MUL_DIV(-9223372036854775808, 1, 1)", i64::MIN),
        ("SELECT MUL_DIV(2 + 3, 4, 2) + 1", 11),
        ("SELECT MUL_DIV(POWER(2, 10), 3, 4)", 768),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn mul_div_rejects_zero_divisors_overflow_and_bad_arguments() {
    for sql in [
        "SELECT MUL_DIV(1, 2, 0)",
        "SELECT MUL_DIV(9223372036854775807, 2, 1)",
        "SELECT MUL_DIV()",
        "SELECT MUL_DIV(1)",
        "SELECT MUL_DIV(1, 2)",
        "SELECT MUL_DIV(1, 2, 3, 4)",
        "SELECT MUL_DIV(1, 2,)",
        "SELECT MUL_DIV(1, 2, 3",
        "SELECT MUL_DIV('x', 2, 3)",
        "SELECT MUL_DIV(1, 2, 3) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
