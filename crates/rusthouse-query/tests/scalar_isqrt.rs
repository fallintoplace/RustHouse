use rusthouse_query::{Projection, parse_select};

#[test]
fn isqrt_returns_exact_integer_floor_square_roots() {
    for (sql, expected) in [
        ("SELECT ISQRT(0)", 0),
        ("SELECT isqrt(1)", 1),
        ("SELECT ISQRT(15)", 3),
        ("SELECT ISQRT(16)", 4),
        ("SELECT ISQRT(9223372036854775807)", 3_037_000_499),
        ("SELECT ISQRT(POWER(3, 4))", 9),
        ("SELECT ISQRT(80 + 1) + 2", 11),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn isqrt_rejects_negative_and_malformed_arguments() {
    for sql in [
        "SELECT ISQRT(-1)",
        "SELECT ISQRT(-9223372036854775808)",
        "SELECT ISQRT()",
        "SELECT ISQRT(1, 2)",
        "SELECT ISQRT(9",
        "SELECT ISQRT('9')",
        "SELECT ISQRT(9) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
