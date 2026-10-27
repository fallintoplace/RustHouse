use rusthouse_query::{Projection, parse_select};

#[test]
fn bit_scans_count_twos_complement_bits() {
    for (sql, expected) in [
        ("SELECT LEADING_ZEROS(0)", 64),
        ("SELECT trailing_zeros(0)", 64),
        ("SELECT LEADING_ZEROS(1)", 63),
        ("SELECT TRAILING_ZEROS(1)", 0),
        ("SELECT LEADING_ZEROS(8)", 60),
        ("SELECT TRAILING_ZEROS(8)", 3),
        ("SELECT LEADING_ZEROS(-1)", 0),
        ("SELECT TRAILING_ZEROS(-8)", 3),
        ("SELECT LEADING_ZEROS(-9223372036854775808)", 0),
        ("SELECT TRAILING_ZEROS(-9223372036854775808)", 63),
        ("SELECT TRAILING_ZEROS(POWER(2, 5)) + 1", 6),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn bit_scans_reject_bad_arity_and_noninteger_arguments() {
    for sql in [
        "SELECT LEADING_ZEROS()",
        "SELECT TRAILING_ZEROS()",
        "SELECT LEADING_ZEROS(1, 2)",
        "SELECT TRAILING_ZEROS(1,)",
        "SELECT LEADING_ZEROS(1",
        "SELECT LEADING_ZEROS('x')",
        "SELECT TRAILING_ZEROS(2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
