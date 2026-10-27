use rusthouse_query::{Projection, parse_select};

#[test]
fn is_prime_identifies_primes_and_composites() {
    for (sql, expected) in [
        ("SELECT IS_PRIME(-7)", 0),
        ("SELECT is_prime(0)", 0),
        ("SELECT IS_PRIME(1)", 0),
        ("SELECT IS_PRIME(2)", 1),
        ("SELECT IS_PRIME(3)", 1),
        ("SELECT IS_PRIME(4)", 0),
        ("SELECT IS_PRIME(25)", 0),
        ("SELECT IS_PRIME(97)", 1),
        ("SELECT IS_PRIME(121)", 0),
        ("SELECT IS_PRIME(2147483647)", 1),
        ("SELECT IS_PRIME(1000000007)", 1),
        ("SELECT IS_PRIME(1000000000000)", 0),
        ("SELECT IS_PRIME(6 * 7) + 1", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn is_prime_rejects_out_of_bound_and_malformed_calls() {
    for sql in [
        "SELECT IS_PRIME(1000000000001)",
        "SELECT IS_PRIME(9223372036854775807)",
        "SELECT IS_PRIME()",
        "SELECT IS_PRIME(1, 2)",
        "SELECT IS_PRIME(1,)",
        "SELECT IS_PRIME(1",
        "SELECT IS_PRIME('x')",
        "SELECT IS_PRIME(2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
