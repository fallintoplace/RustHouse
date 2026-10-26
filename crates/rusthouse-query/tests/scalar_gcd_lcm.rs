use rusthouse_query::{Projection, parse_select};

#[test]
fn gcd_and_lcm_accept_negative_zero_and_composed_constants() {
    for (sql, expected) in [
        ("SELECT GCD(42, 30)", 6),
        ("SELECT gcd(-42, 30)", 6),
        ("SELECT LCM(-6, 8)", 24),
        ("SELECT lcm(0, -9223372036854775808)", 0),
        ("SELECT gcd(0, 0)", 0),
        ("SELECT gcd(-9223372036854775808, 8)", 8),
        ("SELECT lcm(3 * 2, 4) + 1", 13),
        ("SELECT gcd(lcm(8, 12), 18)", 6),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn gcd_and_lcm_reject_unrepresentable_values_and_bad_arity() {
    for sql in [
        "SELECT GCD(-9223372036854775808, 0)",
        "SELECT LCM(-9223372036854775808, 1)",
        "SELECT LCM(9223372036854775807, 2)",
        "SELECT GCD()",
        "SELECT GCD(1)",
        "SELECT LCM(1, 2, 3)",
        "SELECT GCD(1,)",
        "SELECT LCM(1, 2",
        "SELECT GCD('x', 1)",
        "SELECT LCM(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
