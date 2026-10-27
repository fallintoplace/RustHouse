use rusthouse_query::{Projection, parse_select};

#[test]
fn pow10_scales_decimal_integer_expressions() {
    for (sql, expected) in [
        ("SELECT POW10(0)", 1),
        ("SELECT pow10(1)", 10),
        ("SELECT POW10(3)", 1000),
        ("SELECT POW10(18)", 1_000_000_000_000_000_000),
        ("SELECT POW10(1 + 2) / 10", 100),
        ("SELECT POW10(POWER(2, 1))", 100),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn pow10_rejects_negative_or_overflowing_exponents_and_bad_calls() {
    for sql in [
        "SELECT POW10(-1)",
        "SELECT POW10(19)",
        "SELECT POW10(4294967296)",
        "SELECT POW10()",
        "SELECT POW10(2, 3)",
        "SELECT POW10(1,)",
        "SELECT POW10(1",
        "SELECT POW10('x')",
        "SELECT POW10(2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
