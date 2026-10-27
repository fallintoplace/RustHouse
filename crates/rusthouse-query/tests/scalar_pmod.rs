use rusthouse_query::{Projection, parse_select};

#[test]
fn pmod_returns_nonnegative_remainders_for_positive_divisors() {
    for (sql, expected) in [
        ("SELECT PMOD(-10, 3)", 2),
        ("SELECT pmod(10, 3)", 1),
        ("SELECT PMOD(0, 5)", 0),
        ("SELECT PMOD(-9223372036854775808, 3)", 1),
        ("SELECT PMOD(-9223372036854775808, 1)", 0),
        ("SELECT 3 + PMOD(-10, 3)", 5),
        ("SELECT PMOD(GCD(20, 6), 3)", 2),
        ("SELECT PMOD(11, 3 + 2)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn pmod_rejects_nonpositive_divisors_and_invalid_calls() {
    for sql in [
        "SELECT PMOD(3, 0)",
        "SELECT PMOD(3, -2)",
        "SELECT PMOD()",
        "SELECT PMOD(1)",
        "SELECT PMOD(1, 2, 3)",
        "SELECT PMOD(1,)",
        "SELECT PMOD(1, 2",
        "SELECT PMOD('bad', 2)",
        "SELECT PMOD(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
    let error = parse_select("SELECT PMOD(1, 0)").unwrap_err().to_string();
    assert!(error.contains("positive"), "{error}");
}
